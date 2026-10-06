---
title: 'jess.shader'
description: 'Typed shader programs, built by ordinary Luau modules.'
weight: 125
extra:
  kind: api
---

{{ api_signature(value="jess.shader: { program, float, vec2, vec3, vec4, mat3, mat4, texture2d, sampler, min, max, clamp, dot, cross, normalize, length, floor, mix, sample }") }}

A shader, written in Luau. `require("@jess/shader")` is the same table as `jess.shader`, and a shader module is an ordinary Luau module that runs once and returns a `ShaderProgram`.

The vertex and fragment functions you hand `shader.program` don't run on the GPU, and they don't run per frame. Each runs exactly once, while `shader.program` does, to write the program down: `shader.dot(normal, light)` computes nothing, it appends one typed `dot` expression and hands back a value naming it. What comes out is a validated program in jess's own shader IR, which jess lowers to whatever the backend wants. You never write WGSL, and you never see a binding index.

```luau
--!strict
local shader = require '@jess/shader'

local program = shader.program {
  parameters = {
    tint = shader.vec4,
    lightDirection = shader.vec3,
  },

  textures = {
    diffuse = shader.texture2d,
  },

  vertex = function(input)
    return {
      position = input.projection * input.view * input.model * shader.vec4(input.position, 1),
      normal = input.normal,
      uv = input.uv,
    }
  end,

  fragment = function(input)
    local albedo = shader.sample(input.diffuse, input.uv)
    local facing =
      shader.dot(shader.normalize(input.normal), shader.normalize(input.lightDirection))

    return albedo * shader.max(facing, 0) * input.tint
  end,
}

print(program) -- ShaderProgram(2 parameters, 1 texture, 2 varyings)

return program
```

{% usage_note(title="What this is, today") %}
- This is the authoring half. Nothing draws with a `ShaderProgram` yet: the renderer that takes one is a later piece of work, and this page will say so when it isn't.
- The expression set is the one the first real materials need, and no more: arithmetic, the functions below, vector construction, swizzles, and texture sampling. There's no negation, comparison, or control flow. They arrive when a renderer needs them, not because GLSL has them.
- The pass inputs (`sceneColor`, `sceneDepth`, `previousPass`) are names the input knows and the lowering can't bind yet. Reading one is an error when the program finishes, not a quiet nothing.
{% end %}

## shader.program

{{ api_signature(value="shader.program(spec: ShaderProgramSpec) -> ShaderProgram") }}

Builds and validates a program.

- **Parameters:** `spec`, a table:
  - `parameters`: names to [types](#types). Values the renderer sets at run time without recompiling anything: `float`, the vectors, the matrices, and `sampler`.
  - `textures`: names to `shader.texture2d`. Each texture is declared with a sampler of the same name, which [sample](#shader-sample) uses.
  - `vertex`: a function taking the vertex [input](#shaderinput). It returns a table: `position`, the clip-space `Vec4f`, and any other named values, which become the fragment stage's inputs.
  - `fragment`: a function taking the fragment input. It returns the color, a `Vec4f`.
- **Returns:** the [ShaderProgram](#shaderprogram).
- **Order:** parameters are declared sorted by name, then textures sorted by name; `vertex` runs; what it returned is passed along, sorted by name; `fragment` runs; the whole program is validated. A Luau table has no order, so jess doesn't pretend yours does: the same module always builds the same program.
- **Names:** a parameter or texture can't be named like a semantic input or a vertex attribute, and a vertex output can't be named like a parameter, a texture, or a semantic, because the input couldn't tell them apart. A vertex output *can* share a vertex attribute's name (`uv = input.uv`): the fragment stage never sees attributes.
- **Errors:** see [Errors](#errors). Also `unknown jess.shader.program option '<key>': expected fragment, parameters, textures, or vertex`, `shader.program needs a vertex function, not nil`, `parameters.<name> must be a shader type such as shader.vec4, not <kind>`, `parameter "<name>" is a Texture2D: textures are declared in textures`, `the vertex function's table has no position`, and `the fragment function returned <kind>: expected a shader value, the Vec4f color`.

`shader.program` validates the program and leaves it as you wrote it. It doesn't fold constants: that rewrites the program, and rewriting is the consumer's decision.

## ShaderInput

What `vertex` and `fragment` are called with. Every name is one of these, and a name it doesn't have is an error listing the ones it does:

| Names | In | Type |
| --- | --- | --- |
| Your `parameters` | both | As declared. |
| Your `textures` | both | `Texture2D`. Sampling is fragment-only. |
| `position`, `normal` | `vertex` | `Vec3f` vertex attributes. |
| `uv` | `vertex` | `Vec2f` vertex attribute. |
| `color` | `vertex` | `Vec4f` vertex attribute. |
| What `vertex` returned, except `position` | `fragment` | The type of the value returned. |
| `model`, `view`, `projection` | both | `Mat4f`: model-to-world, world-to-view, view-to-clip. |
| `cameraPosition` | both | `Vec3f`, in world space. |
| `viewportSize` | both | `Vec2f`, in pixels. |
| `time` | both | `Float`, seconds since the renderer started. |

A vertex attribute is part of the program's vertex layout only if something reads it. Reading one in `fragment` fails with `vertex input "normal" is only readable in the vertex function; return it from there to pass it to the fragment function as a varying`, which is also the fix.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  parameters = { fogDensity = shader.float },
  vertex = function(input)
    print(input.position, input.model, input.fogDensity)

    return { position = shader.vec4(input.position, 1), depth = input.position.z }
  end,

  fragment = function(input)
    print(input.depth, input.time, input)

    return shader.vec4(input.depth * input.fogDensity)
  end,
}
-- ShaderExpr(Vec3f)  ShaderExpr(Mat4f)  ShaderExpr(Float)
-- ShaderExpr(Float)  ShaderExpr(Float)  ShaderInput(fragment)
```

## ShaderExpr

A symbolic value in the program under construction: an expression, its type, and the program it belongs to. It isn't a number and it isn't a GPU value; there's nothing in it to read.

- **Operators:** `+`, `-`, `*`, `/`, between two values of one numeric type, or a vector and a `Float`. `+` and `-` also take two matrices of one size; `*` also takes a matrix and a `Float`, a matrix and a vector of its size, or two matrices of one size. `input.projection * input.view * input.model * position` multiplies left to right, as Luau evaluates it.
- **Numbers:** a Luau number where a value goes is a `Float` constant, on either side: `2 * color`, `shader.max(facing, 0)`. There are no integer constants yet.
- **Swizzles:** `.x`, `.xy`, `.zyx`, `.wzyx`: one to four of `x`, `y`, `z`, and `w`, giving a `Float` or a vector. They're the only names a value has.
- **`==`:** whether two handles name the same expression. Reading `input.tint` twice gives equal values.
- **`tostring`:** `ShaderExpr(Vec4f)`.
- **Lifetime:** a value belongs to one `shader.program` call. Used after that call returns, it fails with `this shader program is finished: its values belong to a shader.program call that has returned`; used inside another program, with `expression <n> belongs to another shader program`.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  parameters = { tint = shader.vec4 },
  vertex = function(input) return { position = shader.vec4(input.position, 1) } end,
  fragment = function(input)
    local tint = input.tint
    print(tint.xyz, tint.w, tint.zyx.xy)
    print(2 * tint, tint / 2, 1 - tint.w)
    print(tint == input.tint, tint == tint.xyzw)

    return shader.vec4(tint.xyz * 0.5, tint.w)
  end,
}
-- ShaderExpr(Vec3f)  ShaderExpr(Float)  ShaderExpr(Vec2f)
-- ShaderExpr(Vec4f)  ShaderExpr(Vec4f)  ShaderExpr(Float)
-- true  false
```

## Types

{{ api_signature(value="shader.float, shader.vec2, shader.vec3, shader.vec4, shader.mat3, shader.mat4, shader.texture2d, shader.sampler: ShaderType") }}

A `ShaderType` is what `parameters` and `textures` declare, and `tostring` gives its name as errors spell it.

| Member | Type | Declares |
| --- | --- | --- |
| `shader.float` | `Float` | A parameter. |
| `shader.vec2`, `shader.vec3`, `shader.vec4` | `Vec2f`, `Vec3f`, `Vec4f` | A parameter. |
| `shader.mat3`, `shader.mat4` | `Mat3f`, `Mat4f` | A parameter. Column-major. |
| `shader.texture2d` | `Texture2D` | A texture, in `textures`. |
| `shader.sampler` | `Sampler` | A parameter: a sampler of its own, for [sample](#shader-sample)'s three-argument form. |

The vector types and `float` are also constructors, inside a stage function:

{{ api_signature(value="shader.vec4(...: ShaderExpr | number) -> ShaderExpr") }}

- `shader.vec2`, `shader.vec3`, `shader.vec4` take floats and smaller vectors whose sizes add up to theirs (`shader.vec4(input.position, 1)`, `shader.vec4(input.uv, 0, 1)`), or one `Float` to repeat (`shader.vec3(0.5)`).
- `shader.float(0.25)` is a `Float` constant, for when a bare number has nothing beside it to make it one.
- The matrices, `texture2d`, and `sampler` can't be constructed: `Mat4f(Float) is invalid: expected a Vec2f, Vec3f, or Vec4f target`.

```luau
--!strict
local shader = require '@jess/shader'

print(shader.vec4, shader.mat3, shader.texture2d) -- Vec4f  Mat3f  Texture2D

shader.program {
  vertex = function(input)
    print(shader.vec4(input.position, 1), shader.vec4(input.uv, 0, 1), shader.vec3(0.5))

    return { position = shader.vec4(input.position, 1) }
  end,

  fragment = function() return shader.vec4(shader.float(0.25)) end,
}
-- ShaderExpr(Vec4f)  ShaderExpr(Vec4f)  ShaderExpr(Vec3f)
```

## Functions

Each takes shader values or numbers and returns a new value. They're only usable inside a stage function: `shader.dot is only usable inside shader.program's vertex or fragment function`.

| Function | Takes | Gives |
| --- | --- | --- |
| `shader.min(a, b)`, `shader.max(a, b)` | Two values of one numeric type, or a vector and a `Float`. | The smaller or larger, per component. |
| `shader.clamp(value, low, high)` | A value and two bounds of its type, or a vector and two `Float`s. | `value` kept within `low..high`, per component. |
| `shader.mix(start, finish, amount)` | Two `Float`s or vectors of one type, and an amount of that type or `Float`. | `start + (finish - start) * amount`. |
| `shader.dot(a, b)` | Two vectors of one size. | A `Float`. |
| `shader.cross(a, b)` | Two `Vec3f`. | A `Vec3f`. |
| `shader.normalize(value)` | A vector. | The vector at length 1. |
| `shader.length(value)` | A `Float` or a vector. | A `Float`. |
| `shader.floor(value)` | A `Float` or float vector. | The greatest integer no larger than each component, with the same type. |

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  parameters = {
    warm = shader.vec3,
    cool = shader.vec3,
    lightDirection = shader.vec3,
  },

  vertex = function(input)
    return { position = shader.vec4(input.position, 1), normal = input.normal }
  end,

  fragment = function(input)
    local normal = shader.normalize(input.normal)
    local facing = shader.dot(normal, input.lightDirection)
    local shade = shader.mix(input.cool, input.warm, shader.clamp(facing, 0, 1))
    print(facing, shade, shader.length(input.normal), shader.cross(normal, input.lightDirection))
    print(shader.min(shade, 0.8), shader.max(input.warm, input.cool))

    return shader.vec4(shade, 1)
  end,
}
-- ShaderExpr(Float)  ShaderExpr(Vec3f)  ShaderExpr(Float)  ShaderExpr(Vec3f)
-- ShaderExpr(Vec3f)  ShaderExpr(Vec3f)
```

### shader.sample

{{ api_signature(value="shader.sample(texture: ShaderExpr, coordinates: ShaderExpr) -> ShaderExpr") }}
{{ api_signature(value="shader.sample(texture: ShaderExpr, sampler: ShaderExpr, coordinates: ShaderExpr) -> ShaderExpr") }}

Reads a texture at `Vec2f` coordinates, to a `Vec4f`.

With two arguments, the texture is one of the program's `textures`, and the sampler is the one declared with it. With three, the sampler is whatever you pass: a `shader.sampler` parameter. The forms differ by how many arguments you give, not by jess guessing what the second one is.

- **Errors:** `sample(texture, uv) takes a texture from this program's textures, whose sampler comes with it; anything else needs its sampler: sample(texture, sampler, uv)`; operands that aren't `(Texture2D, Sampler, Vec2f)`; and, when the program finishes, `sample is used in the vertex stage: textures are sampled in the fragment stage`.

```luau
--!strict
local shader = require '@jess/shader'

local program = shader.program {
  parameters = { pixelated = shader.sampler },
  textures = { diffuse = shader.texture2d },
  vertex = function(input) return { position = shader.vec4(input.position, 1), uv = input.uv } end,
  fragment = function(input)
    local smooth = shader.sample(input.diffuse, input.uv)
    local blocky = shader.sample(input.diffuse, input.pixelated, input.uv)

    return shader.mix(smooth, blocky, 0.5)
  end,
}

print(program.textures[1], #program.parameters) -- diffuse  0
```

## ShaderProgram

What `shader.program` returns and a shader module hands back: finished, validated, and read-only.

| Field | Type | Holds |
| --- | --- | --- |
| `parameters` | `{ { name: string, type: string } }` | The value parameters, sorted by name. Textures and samplers aren't in it. |
| `textures` | `{ string }` | The texture names, sorted. |
| `varyings` | `{ { name: string, type: string } }` | What `vertex` passes to `fragment`, sorted by name. |

`tostring(program)` counts them: `ShaderProgram(2 parameters, 1 texture, 2 varyings)`.

```luau
--!strict
local shader = require '@jess/shader'

local program = shader.program {
  parameters = { tint = shader.vec4 },
  vertex = function(input)
    return { position = shader.vec4(input.position, 1), color = input.color }
  end,
  fragment = function(input) return input.color * input.tint end,
}

for _, parameter in program.parameters do
  print(parameter.name, parameter.type) -- tint  Vec4f
end

for _, varying in program.varyings do
  print(varying.name, varying.type) -- color  Vec4f
end

print(#program.textures, program) -- 0  ShaderProgram(1 parameter, 0 textures, 1 varying)
```

## Errors

A mistake in an operation fails at the call that made it, in shader terms, with the types involved and the line it's written on. A mistake only the whole program shows (a position that isn't a `Vec4f`, a texture sampled in the vertex stage) is reported when `shader.program` finishes, every problem on its own line, each where its value was written.

This is `shaders/errors.luau`:

```luau
--!strict
local shader = require '@jess/shader'

-- The dot product is wrong on purpose; pcall hands back the message.
print(pcall(function()
  return shader.program {
    parameters = { tint = shader.vec4 },
    vertex = function(input)
      return { position = shader.vec4(input.position, 1), normal = input.normal }
    end,

    fragment = function(input) return shader.vec4(shader.dot(input.normal, input.tint)) end,
  }
end))

-- Two mistakes only the whole program shows: both are listed.
print(pcall(function()
  return shader.program {
    vertex = function(input) return { position = input.position } end,
    fragment = function(input) return input.cameraPosition end,
  }
end))
```

And this is `jess run shaders/errors.luau`:

```text
false	shaders/errors.luau:12 (shaders/errors.luau): dot(Vec3f, Vec4f) is invalid: expected two float vectors of the same size
false	shaders/errors.luau: the shader program is invalid
shaders/errors.luau:19 (shaders/errors.luau): the vertex position is Vec3f: expected Vec4f
shaders/errors.luau:20 (shaders/errors.luau): fragment output 0 is Vec3f: expected Vec4f
```

Each location is `file:line`, then the module that called `shader.program` in parentheses, which differs when a helper module builds part of the program. There's no column: Luau doesn't record one, and jess doesn't make one up. The line is that of the nearest Luau function on the call stack, so a call through `pcall` still points at your source.

Nothing here is a WGSL error. If the backend ever rejects what jess generated, that's jess's bug, not yours.

## Types under `--!strict`

`require("@jess/shader")` is typed as `ShaderModule` (its stub in the folder `jess luau-types` writes is `return jess.shader`), so a shader module type-checks with no annotations: `input` is a `ShaderInput`, every name on it a `ShaderExpr`, and a misspelled function or a value used as a plain number is an error in your editor before it's one at run time. What the checker can't see is a value's shader type: `Vec3f` and `Vec4f` are both `ShaderExpr` to Luau, and the difference is reported when the module runs, at the line.

## Loading a shader module

A shader module is loaded under a contract: run it, and it must return a `ShaderProgram`. From Rust that's `jess_lua::scripts::load_module::<LuaShaderProgram>(&runtime, path)`, which resolves `path` the way `jess run` and `require` do (a file, else a name in the load order's VFS) and gives back the program to fold and lower. A module that returns anything else fails with the module and what came back:

```text
render/materials/water.luau: expected the module to return a jess.shader.Program, but it returned a number
```

`nil`, a string, a table, and another kind of userdata (`a jess.Plugin`) are all named the same way. The contract isn't shader-specific: it's how jess will load anything a module is supposed to *be*.

Related: [require](@/docs/api/jess.md#require).
