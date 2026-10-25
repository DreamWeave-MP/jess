---
title: 'jess.shader'
description: 'Typed shader programs, built by ordinary Luau modules.'
weight: 45
extra:
  kind: api
---

{{ api_signature(value="jess.shader: { program, struct, array, float, int, uint, vec2, vec3, vec4, vec2i, vec3i, vec4i, vec2u, vec3u, vec4u, mat3, mat4, texture2d, texture2dArray, textureCube, texture3d, sampler, min, max, clamp, abs, sign, floor, ceil, round, trunc, fract, sqrt, inverseSqrt, exp, exp2, log, log2, pow, sin, cos, tan, asin, acos, atan, atan2, radians, degrees, saturate, step, smoothstep, mix, dot, cross, length, distance, normalize, reflect, refract, transpose, determinant, dpdx, dpdy, fwidth, equal, notEqual, less, lessEqual, greater, greaterEqual, logicalAnd, logicalOr, logicalNot, select, bitAnd, bitOr, bitXor, bitNot, shiftLeft, shiftRight, sample }") }}

A shader, written in Luau. `require("@jess/shader")` is the same table as `jess.shader`, and a shader module is an ordinary Luau module that runs once and returns a `ShaderProgram`.

The vertex and fragment functions you hand `shader.program` don't run on the GPU, and they don't run per frame. Each runs exactly once, while `shader.program` does, to write the program down: `shader.dot(normal, light)` computes nothing, it appends one typed `dot` expression and hands back a value naming it. What comes out is a validated program in jess's own shader IR, which jess lowers to whatever the backend wants. You never write WGSL, and you never see a binding index.

A program declares everything it reads. Nothing is built in: no camera, no model matrix, no time. A program that wants a camera declares one, and so does every other program that wants one, which is what a shared module of declarations is for.

```luau
--!strict
local shader = require '@jess/shader'

local Camera = shader.struct('Camera', {
  projection = shader.mat4,
  view = shader.mat4,
})

local program = shader.program {
  attributes = {
    position = shader.vec3,
    normal = shader.vec3,
    uv = shader.vec2,
  },

  pass = {
    camera = Camera,
    lightDirection = shader.vec3,
  },

  material = {
    tint = shader.vec4,
    diffuse = shader.texture2d,
  },

  draw = {
    model = shader.mat4,
  },

  vertex = function(input)
    local camera, vertex = input.pass.camera, input.attributes
    local world = input.draw.model * shader.vec4(vertex.position, 1)

    return {
      position = camera.projection * camera.view * world,
      normal = vertex.normal,
      uv = vertex.uv,
    }
  end,

  fragment = function(input)
    local albedo = shader.sample(input.material.diffuse, input.uv)
    local facing =
      shader.dot(shader.normalize(input.normal), shader.normalize(input.pass.lightDirection))

    return albedo * shader.max(facing, 0) * input.material.tint
  end,
}

print(program) -- ShaderProgram(3 attributes, 4 uniforms, 1 texture, 2 varyings)

return program
```

{% usage_note(title="What this is, today") %}
- This is the authoring half. Nothing draws with a `ShaderProgram` yet: the renderer that takes one is a later piece of work, and this page will say so when it isn't.
- The expression set is the one the first real materials need, and no more: arithmetic, the functions below, comparisons and `select`, vector construction, swizzles, and texture sampling. There's no control flow yet. They arrive when a renderer needs them, not because GLSL has them.
{% end %}

## shader.program

{{ api_signature(value="shader.program(spec: ShaderProgramSpec) -> ShaderProgram") }}

Builds and validates a program.

- **Parameters:** `spec`, a table:
  - `attributes`: names to [types](#types). What each vertex has, read from its vertex buffer: see [Attributes](#attributes).
  - `instance`: names to types. What each instance has, read from the instance buffer.
  - `pass`, `material`, `draw`: names to types. What each pass, material, and draw binds: values, structs, and arrays, textures, and samplers. See [Bindings](#bindings).
  - `vertex`: a function taking the vertex [input](#shaderinput). It returns a table: `position`, the clip-space `Vec4f`, and any other named values, which become the fragment stage's inputs.
  - `fragment`: a function taking the fragment input. It returns the color, a `Vec4f`.
- **Returns:** the [ShaderProgram](#shaderprogram).
- **Order:** attributes are declared sorted by name, then instance attributes, then what the pass, the material, and the draw bind, each sorted by name; `vertex` runs; what it returned is passed along, sorted by name; `fragment` runs; the whole program is validated. A Luau table has no order, so jess doesn't pretend yours does: the same module always builds the same program.
- **Names:** every declaration lives under its own name (`input.attributes.uv`, `input.material.tint`), so names only clash within one table. A vertex output can't be named `attributes`, `instance`, `pass`, `material`, or `draw`, because the fragment input couldn't tell it apart from the namespace.
- **Errors:** see [Errors](#errors). Also `unknown jess.shader.program option '<key>': expected attributes, draw, fragment, instance, material, pass, or vertex`, `shader.program needs a vertex function, not nil`, `material.<name> must be a shader type such as shader.vec4, not <kind>`, `attributes.<name> must be a value type such as shader.vec3, not Texture2D`, `the vertex function's table has no position`, and `the fragment function returned <kind>: expected a shader value, the Vec4f color`.

`shader.program` validates the program and leaves it as you wrote it. It doesn't fold constants: that rewrites the program, and rewriting is the consumer's decision.

## ShaderInput

What `vertex` and `fragment` are called with. Every name is one of these, and a name it doesn't have is an error listing the ones it does:

| Names | In | Type |
| --- | --- | --- |
| `attributes` | `vertex` | Your `attributes`, by name: `input.attributes.position`. |
| `instance` | `vertex` | Your `instance` attributes, by name: `input.instance.transform`. |
| `pass`, `material`, `draw` | both | What each binds, by name: `input.material.tint`. See [Bindings](#bindings). |
| `builtin` | both | What the pipeline gives the stage: `input.builtin.vertexIndex`. See [Built-in values](#built-in-values). |
| What `vertex` returned, except `position` | `fragment` | The type of the value returned. |

The fragment function has no attributes: `input.attributes` there fails with `the attributes are only readable in the vertex function; return what the fragment function needs from them, to pass it as a varying`, which is also the fix.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  attributes = { position = shader.vec3 },
  pass = { time = shader.float },
  material = { fogDensity = shader.float },
  draw = { model = shader.mat4 },
  vertex = function(input)
    local position = input.attributes.position
    print(position, input.draw.model, input.material.fogDensity)

    return { position = input.draw.model * shader.vec4(position, 1), depth = position.z }
  end,

  fragment = function(input)
    print(input.depth, input.pass.time, input)

    return shader.vec4(input.depth * input.material.fogDensity)
  end,
}
-- ShaderExpr(Vec3f)  ShaderExpr(Mat4f)  ShaderExpr(Float)
-- ShaderExpr(Float)  ShaderExpr(Float)  ShaderInput(fragment)
```

### Built-in values

`input.builtin` holds what the pipeline gives a stage rather than what a buffer does. Each is read in the stage that has it; reading one in the other fails at once, `builtin vertexIndex is only readable in the vertex function`.

| Name | In | Type | What |
| --- | --- | --- | --- |
| `vertexIndex` | `vertex` | `UInt` | The vertex's index in the draw; with an index buffer, the index it read. |
| `instanceIndex` | `vertex` | `UInt` | The instance's index in the draw. |
| `position` | `fragment` | `Vec4f` | Where the fragment is: `x` and `y` in pixels from the target's top left, at the pixel's centre, `z` its depth, and `w` one over the clip position's `w`. |
| `frontFacing` | `fragment` | `Bool` | Whether the fragment belongs to a triangle facing the camera. |

A program with no attributes at all can still draw: three vertices and `vertexIndex` make a triangle that covers the target, the usual start of a full-screen pass.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  vertex = function(input)
    local index = input.builtin.vertexIndex
    local corner = shader.vec2(
      shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2))),
      shader.float(shader.bitAnd(index, shader.uint(2)))
    )
    print(input.builtin, index, corner)

    return { position = shader.vec4(corner * 2 - 1, 0, 1) }
  end,

  fragment = function(input)
    local pixel = input.builtin.position
    local stripe = shader.step(0.5, shader.fract(pixel.x / 8))
    print(pixel, input.builtin.frontFacing)

    return shader.select(input.builtin.frontFacing, shader.vec4(stripe), shader.vec4(0, 0, 0, 1))
  end,
}
-- ShaderBuiltins(vertex)  ShaderExpr(UInt)  ShaderExpr(Vec2f)
-- ShaderExpr(Vec4f)  ShaderExpr(Bool)
```

## Attributes

`attributes` are what each vertex has: its position, its normal, its texture coordinates, whatever the mesh carries. `instance` attributes are what each instance has, for drawing one mesh many times in one go: a transform, a tint, an index into something. Both are declared by name and [type](#types), and both are read in the vertex function only, from `input.attributes` and `input.instance`.

- **Types:** `float`, `int`, `uint`, their vectors, and `mat3` and `mat4`. A matrix takes one vertex location per column on the way in, and you read it whole.
- **Layout:** jess packs each rate's attributes into one buffer record, in declaration order (sorted by name), with no padding, and reports where every one is. Whatever fills the vertex and instance buffers writes them from that, so nobody counts bytes by hand.
- **Limit:** the attributes of both rates take at most 16 locations together.
- **Errors:** `the attributes have no "normals": they have normal, position`; `the instance attributes have no "transform": the program declares none`; and, when the program finishes, `the attributes take 20 locations: at most 16`.

```luau
--!strict
local shader = require '@jess/shader'

local grass = shader.program {
  attributes = { position = shader.vec3 },
  instance = { transform = shader.mat4, tint = shader.vec4 },
  pass = { viewProjection = shader.mat4 },
  vertex = function(input)
    local placed = input.instance.transform * shader.vec4(input.attributes.position, 1)
    print(input.attributes, input.instance.transform)

    return { position = input.pass.viewProjection * placed, tint = input.instance.tint }
  end,

  fragment = function(input) return input.tint end,
}

for _, attribute in grass.instance do
  print(attribute.name, attribute.type)
end
-- ShaderAttributes(attributes)  ShaderExpr(Mat4f)
-- tint  Vec4f
-- transform  Mat4f
```

## Bindings

Everything else a program reads is bound by one of three frequencies, which say how often it changes and so who supplies it:

| Frequency | Bound | For |
| --- | --- | --- |
| `pass` | Once per pass | What every draw in it shares: a camera, a light list, a shadow map, the previous pass's output. |
| `material` | With the material | What every draw of one material shares: a tint, a texture, a roughness. |
| `draw` | Per draw | What one draw has alone: its transform, its own lookup table. |

Each frequency's table names what it binds:

- **Values, structs, and arrays** are its uniform block's members. A value is `shader.vec4` and the rest; a struct is a type from [shader.struct](#shader-struct); an array is a type from [shader.array](#shader-array). Change one, and nothing recompiles: it's data.
- **`shader.texture2d`**, **`shader.texture2dArray`**, **`shader.textureCube`**, and **`shader.texture3d`** are textures, each declared with a sampler of the same name, which [sample](#shader-sample) uses.
- **`shader.sampler`** is a sampler of its own, for `sample`'s three-argument form.

Reading goes one step at a time. `input.pass.camera` is the camera; `.view` is its view matrix, a value. An array is indexed from 0 by a number or by a shader value, `#` is its length, and an element is a value or a struct to go on into: `input.pass.lights[i].color`. A step that reaches a value reads it; a texture or sampler is a handle for `sample`. Until then it's a `ShaderPlace`, which isn't a value: `input.pass.camera + 1` fails with `+: expected a shader value, not Camera, a struct: read one of its fields`.

Where it all lives is jess's decision, never yours. Each frequency is one bind group, its uniform block first and its textures and samplers after it, and every member is laid out by its alignment, largest first, so nothing pads what follows it. The layout is reported with the lowered program, so whatever writes the buffers writes them from that. One rule of the layout shows: in a uniform block, an array's elements must be a multiple of 16 bytes apart, so a `shader.array(shader.float, 4)` there fails when the program finishes, with `uniform material.weights is an array of elements 4 bytes apart`. Use `shader.vec4` elements, or a struct.

- **Errors:** `the material has no "tnit": it has diffuse, tint`; `Light has no field "colour": it has color, direction`; `Light[4] has no element 4: its elements are 0 to 3`; `Light[4] is indexed by a number or a shader value, not string`; `Camera has no length: only an array does`.

```luau
--!strict
local shader = require '@jess/shader'

local Light = shader.struct('Light', { color = shader.vec3, direction = shader.vec3 })

shader.program {
  attributes = { position = shader.vec3, normal = shader.vec3 },
  pass = { lights = shader.array(Light, 4), viewProjection = shader.mat4 },
  vertex = function(input)
    local position = shader.vec4(input.attributes.position, 1)

    return { position = input.pass.viewProjection * position, normal = input.attributes.normal }
  end,

  fragment = function(input)
    local lights = input.pass.lights
    print(lights, #lights, lights[0], lights[0].color)

    local light = lights[0]
    local facing = shader.max(shader.dot(shader.normalize(input.normal), light.direction), 0)

    return shader.vec4(light.color * facing, 1)
  end,
}
-- ShaderPlace(Light[4])  4  ShaderPlace(Light)  ShaderExpr(Vec3f)
```

### shader.struct

{{ api_signature(value="shader.struct(name: string, fields: { [string]: ShaderType }) -> ShaderType") }}

A struct type: named fields of values, structs, and arrays. The name is what errors and reflection call it. A struct is a type, not a declaration: a program declares it where it's used, so one module can hold the structs a dozen programs share.

- **Errors:** `shader.struct takes a name and a table of fields, not number`; `Bad.tex is a Texture2D: textures and samplers are bound, not held in a struct or array`.

### shader.array

{{ api_signature(value="shader.array(element: ShaderType, length: number) -> ShaderType") }}

An array type: `length` values or structs, at least one. An array holds no arrays: put one in a struct to nest it.

- **Errors:** `shader.array's length must be a whole number of at least 1, not 0`; `shader.array's element is an array: an array holds values or structs, not arrays`.

## ShaderExpr

A symbolic value in the program under construction: an expression, its type, and the program it belongs to. It isn't a number and it isn't a GPU value; there's nothing in it to read.

- **Operators:** `+`, `-`, `*`, `/`, and `%`, between two values of one numeric type, or a vector and a scalar of its kind (a `Vec3f` and a `Float`, a `Vec2u` and a `UInt`). `%` is what's left of truncated division, so it has the sign of its left side: `-7 % 3` is `-1`, where Luau's own `%` gives `2`. Unary `-` negates a `Float`, an `Int`, or a vector of either; there's no negating a `UInt` or a matrix. `+` and `-` also take two matrices of one size; `*` also takes a matrix and a `Float`, a matrix and a vector of its size, or two matrices of one size. `projection * view * model * position` multiplies left to right, as Luau evaluates it.
- **Numbers:** a Luau number where a value goes is a `Float` constant, on either side: `2 * color`, `shader.max(facing, 0)`. Integers are spelled `shader.int(3)` and `shader.uint(3)`, so `2 * cell` is a `Float` times whatever `cell` is, and fails for an integer vector. A Luau `true` or `false` is a `Bool` constant.
- **Swizzles:** `.x`, `.xy`, `.zyx`, `.wzyx`: one to four of `x`, `y`, `z`, and `w`, giving a scalar or a vector of the same kind. They're the only names a value has.
- **`==`:** whether two handles name the same expression. Reading `input.material.tint` twice gives equal values.
- **`tostring`:** `ShaderExpr(Vec4f)`.
- **Lifetime:** a value belongs to one `shader.program` call. Used after that call returns, it fails with `this shader program is finished: its values belong to a shader.program call that has returned`; used inside another program, with `expression <n> belongs to another shader program`.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  attributes = { position = shader.vec3 },
  material = { tint = shader.vec4 },
  vertex = function(input) return { position = shader.vec4(input.attributes.position, 1) } end,
  fragment = function(input)
    local tint = input.material.tint
    print(tint.xyz, tint.w, tint.zyx.xy)
    print(2 * tint, tint / 2, 1 - tint.w)
    print(tint == input.material.tint, tint == tint.xyzw)

    return shader.vec4(tint.xyz * 0.5, tint.w)
  end,
}
-- ShaderExpr(Vec3f)  ShaderExpr(Float)  ShaderExpr(Vec2f)
-- ShaderExpr(Vec4f)  ShaderExpr(Vec4f)  ShaderExpr(Float)
-- true  false
```

## Types

{{ api_signature(value="shader.float, shader.int, shader.uint, shader.vec2, shader.vec3, shader.vec4, shader.vec2i, shader.vec3i, shader.vec4i, shader.vec2u, shader.vec3u, shader.vec4u, shader.mat3, shader.mat4, shader.texture2d, shader.texture2dArray, shader.textureCube, shader.texture3d, shader.sampler: ShaderType") }}

A `ShaderType` is what attributes and bindings declare, and `tostring` gives its name as errors spell it.

| Member | Type | Declares |
| --- | --- | --- |
| `shader.float`, `shader.int`, `shader.uint` | `Float`, `Int`, `UInt` | An attribute, a uniform member, a field, or an element. |
| `shader.vec2`, `shader.vec3`, `shader.vec4` | `Vec2f`, `Vec3f`, `Vec4f` | The same. |
| `shader.vec2i`, `shader.vec3i`, `shader.vec4i` | `Vec2i`, `Vec3i`, `Vec4i` | The same: signed integers. |
| `shader.vec2u`, `shader.vec3u`, `shader.vec4u` | `Vec2u`, `Vec3u`, `Vec4u` | The same: unsigned integers. |
| `shader.mat3`, `shader.mat4` | `Mat3f`, `Mat4f` | The same. Column-major. |
| `shader.texture2d` | `Texture2D` | A texture and its sampler, in `pass`, `material`, or `draw`. |
| `shader.texture2dArray` | `Texture2DArray` | The same: layers of 2D textures of one size. |
| `shader.textureCube` | `TextureCube` | The same: six square faces, an environment or a sky. |
| `shader.texture3d` | `Texture3D` | The same: a volume. |
| `shader.sampler` | `Sampler` | A sampler of its own, for [sample](#shader-sample)'s three-argument form. |
| [shader.struct](#shader-struct)`(...)` | its name | A uniform member, a field, or an element. |
| [shader.array](#shader-array)`(...)` | `Element[length]` | A uniform member or a field. |

The value types are also constructors, inside a stage function:

{{ api_signature(value="shader.vec4(...: ShaderExpr | number) -> ShaderExpr") }}

- `shader.vec2`, `shader.vec3`, `shader.vec4` take floats and smaller vectors whose sizes add up to theirs (`shader.vec4(position, 1)`, `shader.vec4(uv, 0, 1)`), or one `Float` to repeat (`shader.vec3(0.5)`). The integer vectors take integers the same way, and a number among them is that vector's integer: `shader.vec3i(1, -2, 3)`, `shader.vec2u(shader.uint(1))`. A number that isn't whole, or is out of range, fails: `shader.vec3i: 1.5 is not an Int: expected a whole number from -2147483648 to 2147483647`.
- `shader.float(0.25)` is a `Float` constant, for when a bare number has nothing beside it to make it one, and `shader.int(-3)` and `shader.uint(7)` are the integer constants.
- One value of another scalar converts, keeping its size: `shader.float(index)` is a `UInt` index as a `Float`, `shader.vec3(cell)` a `Vec3u` as a `Vec3f`, and `shader.vec3i(cell)` the same cell signed. A float converts to an integer by truncating toward zero. A conversion never changes the size: `shader.float(tint)` fails with `Float(Vec4f) is invalid: expected a scalar or vector of the same size`; take a component first.
- `shader.mat3` and `shader.mat4` take their columns: `shader.mat3(right, up, forward)`, three `Vec3f`s. Anything else fails: `Mat4f(Float) is invalid: expected four Vec4f columns`.
- A texture, a sampler, a struct, or an array can't be constructed: `Camera can't be constructed: it is declared, and read from the input`.

```luau
--!strict
local shader = require '@jess/shader'

print(shader.vec4, shader.mat3, shader.texture2d) -- Vec4f  Mat3f  Texture2D
print(shader.struct('Fog', { density = shader.float }), shader.array(shader.vec4, 8))
-- Fog  Vec4f[8]

shader.program {
  attributes = { cell = shader.vec3u, position = shader.vec3, uv = shader.vec2 },
  vertex = function(input)
    local vertex = input.attributes
    print(shader.vec4(vertex.position, 1), shader.vec4(vertex.uv, 0, 1), shader.vec3(0.5))
    print(shader.vec3i(1, -2, 3), shader.uint(7), shader.vec2u(shader.uint(1)).yx)
    print(shader.vec3(vertex.cell), shader.float(vertex.cell.x))
    print(shader.mat3(vertex.position, shader.vec3(0, 1, 0), shader.vec3(0, 0, 1)))

    return { position = shader.vec4(vertex.position, 1) }
  end,

  fragment = function() return shader.vec4(shader.float(0.25)) end,
}
-- ShaderExpr(Vec4f)  ShaderExpr(Vec4f)  ShaderExpr(Vec3f)
-- ShaderExpr(Vec3i)  ShaderExpr(UInt)  ShaderExpr(Vec2u)
-- ShaderExpr(Vec3f)  ShaderExpr(Float)
-- ShaderExpr(Mat3f)
```

## Functions

Each takes shader values or numbers and returns a new value. They're only usable inside a stage function: `shader.dot is only usable inside shader.program's vertex or fragment function`. The names are WGSL's, and so is what each computes.

| Function | Takes | Gives |
| --- | --- | --- |
| `shader.min(a, b)`, `shader.max(a, b)` | Two values of one numeric type, or a vector and a scalar of its kind. | The smaller or larger, per component. |
| `shader.clamp(value, low, high)` | A value and two bounds of its type, or a vector and two scalars of its kind. | `value` kept within `low..high`, per component. |
| `shader.abs(value)` | An `Int`, `UInt`, `Float`, or a vector of one. | The absolute value, per component. |
| `shader.sign(value)` | An `Int`, `Float`, or a vector of one. | -1, 0, or 1, per component. |
| `shader.floor`, `ceil`, `round`, `trunc` | A `Float` or float vector. | Each component rounded down, up, to the nearest integer (a half to the even one), or toward zero. |
| `shader.fract(value)` | A `Float` or float vector. | `value - floor(value)`. |
| `shader.sqrt`, `inverseSqrt`, `exp`, `exp2`, `log`, `log2` | A `Float` or float vector. | The root, one over it, `e` or 2 to the power, or the natural or base-2 logarithm, per component. |
| `shader.pow(base, exponent)` | Two `Float`s or float vectors of one type, or a float vector and a `Float`. | `base` to the power `exponent`, per component. |
| `shader.sin`, `cos`, `tan`, `asin`, `acos`, `atan` | A `Float` or float vector, in radians. | The function, per component; the inverses give radians. |
| `shader.atan2(y, x)` | As `pow`. | The angle of the point `(x, y)`, from -π to π. |
| `shader.radians(degrees)`, `shader.degrees(radians)` | A `Float` or float vector. | The angle in the other unit. |
| `shader.saturate(value)` | A `Float` or float vector. | `clamp(value, 0, 1)`. |
| `shader.step(edge, value)` | As `pow`. | 0 where `value < edge`, else 1, per component. |
| `shader.smoothstep(low, high, value)` | A `Float` or float vector, and two edges of its type or `Float`s. | 0 at `low`, 1 at `high`, and a smooth curve between. |
| `shader.mix(start, finish, amount)` | Two `Float`s or vectors of one type, and an amount of that type or `Float`. | `start + (finish - start) * amount`. |
| `shader.dot(a, b)` | Two vectors of one type. | Their component: a `Float`, `Int`, or `UInt`. |
| `shader.cross(a, b)` | Two `Vec3f`. | A `Vec3f`. |
| `shader.length(value)` | A `Float` or a vector. | A `Float`. |
| `shader.distance(a, b)` | Two `Float`s or float vectors of one type. | `length(a - b)`. |
| `shader.normalize(value)` | A float vector. | The vector at length 1. |
| `shader.reflect(incident, normal)` | Two float vectors of one type; `normal` at length 1. | `incident` reflected about the plane `normal` faces out of. |
| `shader.refract(incident, normal, eta)` | Two float vectors of one type, and a `Float`. | `incident` bent through that plane at the ratio of refractive indices `eta`, or zero where it reflects entirely. |
| `shader.transpose(matrix)` | A matrix. | Its rows as its columns. |
| `shader.determinant(matrix)` | A matrix. | A `Float`. |
| `shader.dpdx`, `dpdy`, `fwidth` | A `Float` or float vector. Fragment stage only. | How fast the value changes from one pixel to the next across the screen, down it, or the sum of both as absolute values. |

A `Float` beside a vector in `pow`, `atan2`, `step`, and `smoothstep` counts for every component: `shader.step(0.5, color.xyz)`. A derivative in the vertex stage fails when the program finishes: `dpdx is used in the vertex stage: derivatives are taken in the fragment stage`.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  attributes = { position = shader.vec3, normal = shader.vec3 },
  material = {
    warm = shader.vec3,
    cool = shader.vec3,
    lightDirection = shader.vec3,
  },

  vertex = function(input)
    local vertex = input.attributes

    return { position = shader.vec4(vertex.position, 1), normal = vertex.normal }
  end,

  fragment = function(input)
    local material = input.material
    local normal = shader.normalize(input.normal)
    local facing = shader.dot(normal, material.lightDirection)
    local shade = shader.mix(material.cool, material.warm, shader.clamp(facing, 0, 1))
    print(facing, shade, shader.length(input.normal), shader.cross(normal, material.lightDirection))
    print(shader.min(shade, 0.8), shader.max(material.warm, material.cool))
    print(shader.smoothstep(0.2, 0.8, facing), shader.step(0.5, shade), shader.fwidth(facing))

    return shader.vec4(shade, 1)
  end,
}
-- ShaderExpr(Float)  ShaderExpr(Vec3f)  ShaderExpr(Float)  ShaderExpr(Vec3f)
-- ShaderExpr(Vec3f)  ShaderExpr(Vec3f)
-- ShaderExpr(Float)  ShaderExpr(Vec3f)  ShaderExpr(Float)
```

### Comparisons and choice

Luau's `==` and `<` answer a Luau boolean, so a shader compares with functions, each giving a `Bool`, and chooses with `select`.

| Function | Takes | Gives |
| --- | --- | --- |
| `shader.equal(a, b)`, `shader.notEqual(a, b)` | Two scalars of one type, `Bool` included. | Whether they're equal, or differ. |
| `shader.less(a, b)`, `shader.lessEqual(a, b)`, `shader.greater(a, b)`, `shader.greaterEqual(a, b)` | Two `Int`s, `UInt`s, or `Float`s of one type. | `a < b`, `a <= b`, `a > b`, `a >= b`. |
| `shader.logicalAnd(a, b)`, `shader.logicalOr(a, b)` | Two `Bool`s. | Whether both hold, or either does. |
| `shader.logicalNot(value)` | A `Bool`. | Whether it doesn't hold. |
| `shader.select(condition, accept, reject)` | A `Bool`, then two scalars or vectors of one type. | `accept` where `condition` holds, else `reject`. |

`select` isn't a branch: both choices are computed, and one is kept. It's the right thing for picking a value, and cheap, since a GPU would run both sides of a short branch anyway.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  attributes = { position = shader.vec3, height = shader.float },
  material = { snowLine = shader.float, snow = shader.vec4, rock = shader.vec4 },
  vertex = function(input)
    local vertex = input.attributes

    return { position = shader.vec4(vertex.position, 1), height = vertex.height }
  end,

  fragment = function(input)
    local material = input.material
    local snowy = shader.greaterEqual(input.height, material.snowLine)
    print(snowy, shader.logicalAnd(snowy, true), shader.logicalNot(snowy))

    return shader.select(snowy, material.snow, material.rock)
  end,
}
-- ShaderExpr(Bool)  ShaderExpr(Bool)  ShaderExpr(Bool)
```

### Bits

The bitwise operators work on `Int` and `UInt` values and their vectors, both sides of one type. Luau has no bitwise operators of its own to borrow, so they're functions.

| Function | Takes | Gives |
| --- | --- | --- |
| `shader.bitAnd(a, b)`, `shader.bitOr(a, b)`, `shader.bitXor(a, b)` | Two `Int` or `UInt` values of one type. | And, or, and exclusive or, bit by bit. |
| `shader.bitNot(value)` | An `Int` or `UInt` value. | Every bit flipped. |
| `shader.shiftLeft(value, amount)`, `shader.shiftRight(value, amount)` | An `Int` or `UInt` value, and a `UInt` amount, or a `UInt` vector of the value's size. | The value shifted; right shifts of an `Int` keep its sign. |

A constant amount of 32 or more fails where it's written, with `a shift by 40 is too far: shift by less than 32`.

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  attributes = { position = shader.vec3, packed = shader.uint },
  vertex = function(input)
    local packed = input.attributes.packed
    local low = shader.bitAnd(packed, shader.uint(0xff))
    local high = shader.shiftRight(packed, shader.uint(8))
    print(low, high, shader.bitNot(low), -input.attributes.position)

    return { position = shader.vec4(input.attributes.position, 1) }
  end,

  fragment = function() return shader.vec4(1) end,
}
-- ShaderExpr(UInt)  ShaderExpr(UInt)  ShaderExpr(UInt)  ShaderExpr(Vec3f)
```

### shader.sample

{{ api_signature(value="shader.sample(texture: ShaderTexture, coordinates: ShaderExpr, layer: (ShaderExpr | number)?) -> ShaderExpr") }}
{{ api_signature(value="shader.sample(texture: ShaderTexture, sampler: ShaderSampler, coordinates: ShaderExpr, layer: (ShaderExpr | number)?) -> ShaderExpr") }}

Reads a texture to a `Vec4f`, at the coordinates its kind takes:

| Texture | Coordinates |
| --- | --- |
| `texture2d` | `Vec2f`, from 0 to 1 across it. |
| `texture2dArray` | `Vec2f`, and then the layer: an `Int` or `UInt`, or a number, which is a `UInt`. |
| `textureCube` | `Vec3f`, a direction from the cube's centre; it needn't be of length 1. |
| `texture3d` | `Vec3f`, from 0 to 1 through it. |

Without a sampler, it's the one declared with the texture. With one, it's whatever sampler you pass: one a frequency binds on its own. A sampler is a handle, so jess tells it from coordinates by what it is, not by how many arguments there are.

- **Errors:** `shader.sample: expected a texture, such as input.material.diffuse, not a Vec4f value`; `shader.sample: expected a sampler, not a Vec4f value`; coordinates or a layer the texture doesn't take, `sample(TextureCube, Sampler, Vec2f) is invalid: expected sample(TextureCube, Sampler, Vec3f)`; and, when the program finishes, `sample is used in the vertex stage: textures are sampled in the fragment stage`.

```luau
--!strict
local shader = require '@jess/shader'

local program = shader.program {
  attributes = { position = shader.vec3, uv = shader.vec2 },
  material = { diffuse = shader.texture2d, pixelated = shader.sampler },
  vertex = function(input)
    return { position = shader.vec4(input.attributes.position, 1), uv = input.attributes.uv }
  end,

  fragment = function(input)
    local material = input.material
    print(material.diffuse, material.pixelated)

    local smooth = shader.sample(material.diffuse, input.uv)
    local blocky = shader.sample(material.diffuse, material.pixelated, input.uv)

    return shader.mix(smooth, blocky, 0.5)
  end,
}

for _, binding in program.material do
  print(binding.name, binding.type)
end
-- ShaderTexture(Texture2D)  ShaderSampler(Sampler)
-- diffuse  Texture2D
-- pixelated  Sampler
```

A sky and a layer of decals, each sampled the way its kind is:

```luau
--!strict
local shader = require '@jess/shader'

shader.program {
  attributes = { position = shader.vec3 },
  material = { sky = shader.textureCube, decals = shader.texture2dArray },
  draw = { decal = shader.uint },
  vertex = function(input)
    local position = input.attributes.position

    return { position = shader.vec4(position, 1), direction = position }
  end,

  fragment = function(input)
    local material = input.material
    local sky = shader.sample(material.sky, input.direction)
    local decal = shader.sample(material.decals, input.direction.xy, input.draw.decal)
    print(material.sky, sky, shader.sample(material.decals, input.direction.xy, 3))

    return shader.mix(sky, decal, decal.w)
  end,
}
-- ShaderTexture(TextureCube)  ShaderExpr(Vec4f)  ShaderExpr(Vec4f)
```

## ShaderProgram

What `shader.program` returns and a shader module hands back: finished, validated, and read-only.

| Field | Type | Holds |
| --- | --- | --- |
| `attributes` | `{ { name: string, type: string } }` | The per-vertex attributes, sorted by name. |
| `instance` | `{ { name: string, type: string } }` | The per-instance attributes, sorted by name. |
| `pass`, `material`, `draw` | `{ { name: string, type: string } }` | What each frequency binds, sorted by name: `Vec4f`, `Camera`, `Light[4]`, `Texture2D`, `Sampler`. A texture's own sampler is part of the texture. |
| `varyings` | `{ { name: string, type: string } }` | What `vertex` passes to `fragment`, sorted by name. |

`tostring(program)` counts them, both kinds of attribute together and the uniform members of every frequency together: `ShaderProgram(3 attributes, 4 uniforms, 1 texture, 2 varyings)`.

```luau
--!strict
local shader = require '@jess/shader'

local program = shader.program {
  attributes = { position = shader.vec3, color = shader.vec4 },
  material = { tint = shader.vec4 },
  vertex = function(input)
    local vertex = input.attributes

    return { position = shader.vec4(vertex.position, 1), color = vertex.color }
  end,
  fragment = function(input) return input.color * input.material.tint end,
}

for _, binding in program.material do
  print(binding.name, binding.type) -- tint  Vec4f
end

for _, varying in program.varyings do
  print(varying.name, varying.type) -- color  Vec4f
end

print(#program.pass, program)
-- 0  ShaderProgram(2 attributes, 1 uniform, 0 textures, 1 varying)
```

## Errors

A mistake in an operation fails at the call that made it, in shader terms, with the types involved and the line it's written on. A mistake only the whole program shows (a position that isn't a `Vec4f`, a texture sampled in the vertex stage) is reported when `shader.program` finishes, every problem on its own line, each where its value was written.

This is `shaders/errors.jsl`:

```luau
--!strict
local shader = require '@jess/shader'

-- The dot product is wrong on purpose; pcall hands back the message.
print(pcall(function()
  return shader.program {
    attributes = { position = shader.vec3, normal = shader.vec3 },
    material = { tint = shader.vec4 },
    vertex = function(input)
      local vertex = input.attributes

      return { position = shader.vec4(vertex.position, 1), normal = vertex.normal }
    end,

    fragment = function(input)
      return shader.vec4(shader.dot(input.normal, input.material.tint))
    end,
  }
end))

-- Two mistakes only the whole program shows: both are listed.
print(pcall(function()
  return shader.program {
    attributes = { position = shader.vec3 },
    pass = { cameraPosition = shader.vec3 },
    vertex = function(input) return { position = input.attributes.position } end,
    fragment = function(input) return input.pass.cameraPosition end,
  }
end))
```

And this is `jess run shaders/errors.jsl`:

```text
false	shaders/errors.jsl:16 (shaders/errors.jsl): dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type
false	shaders/errors.jsl: the shader program is invalid
shaders/errors.jsl:26 (shaders/errors.jsl): the vertex position is Vec3f: expected Vec4f
shaders/errors.jsl:27 (shaders/errors.jsl): fragment output 0 is Vec3f: expected Vec4f
```

Each location is `file:line`, then the module that called `shader.program` in parentheses, which differs when a helper module builds part of the program. There's no column: Luau doesn't record one, and jess doesn't make one up. The line is that of the nearest Luau function on the call stack, so a call through `pcall` still points at your source.

Nothing here is a WGSL error. If the backend ever rejects what jess generated, that's jess's bug, not yours.

## Types under `--!strict`

`require("@jess/shader")` is typed as `ShaderModule` (its stub in the folder `jess luau-types` writes is `return jess.shader`), so a shader module type-checks with no annotations: `input` is a `ShaderInput`, every name on it a `ShaderExpr`, and a misspelled function or a value used as a plain number is an error in your editor before it's one at run time. What the checker can't see is a value's shader type, or whether a name a binding has is a value, a struct, an array, or a texture: to Luau they're all `ShaderExpr`, indexable and measurable, and the difference is reported when the module runs, at the line.

## Loading a shader module

A shader module is loaded under a contract: run it, and it must return a `ShaderProgram`. From Rust that's `jess::lua::scripts::load_module::<LuaShaderProgram>(&runtime, path)`, which runs the file and gives back the program to fold and lower. A module that returns anything else fails with the module and what came back:

```text
render/materials/water.jsl: expected the module to return a jess.shader.Program, but it returned a number
```

`nil`, a string, a table, and another kind of userdata are all named the same way. The contract isn't shader-specific: it's how jess will load anything a module is supposed to *be*.

Related: [require](@/docs/api/jess.md#require).
