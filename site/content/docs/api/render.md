---
title: '@jess/render'
description: 'Fill the buffers a shader program reads by name, from the layout jess gives it, and use the standard camera: its block, its projections, and its view.'
weight: 51
extra:
  kind: api
---

{{ api_signature(value="require('@jess/render'): { block, vertices, elements }") }}

`@jess/render` is what drawing means, in Luau, over [`jess.gpu`](@/docs/api/gpu.md). The kernel lays every [shader program](@/docs/api/shader.md) out and binds by binding number; this fills those layouts by name. Nothing here decides where a member goes: `program.layout` does, and a writer only reads it, so a block written here is always the block the program reads.

## Writers

{{ api_signature(value="render.block(layout: ShaderLayout, frequency: 'pass' | 'material' | 'draw') -> Block") }}

The uniform block `frequency` declares, as bytes, all zeros. `block:set(name, value)` writes a member where the layout puts it, and `block:upload(gpuBuffer, offset?)` writes the bytes to a buffer. `block.bytes` and `block.size` are the bytes and their length.

{{ api_signature(value="render.vertices(layout: ShaderLayout, count: number, rate: ('vertex' | 'instance')?) -> Vertices") }}

`count` vertices of the program's per-vertex attributes, or instances of its per-instance ones. `vertices:set(index, name, value)` writes vertex `index` (from 1); `stride`, `count`, `bytes`, and `upload` are a block's.

{{ api_signature(value="render.elements(layout: ShaderLayout, frequency, name: string, count: number) -> Elements") }}

`count` elements of the storage buffer `name`. `elements:set(index, value)` writes element `index`, from 1.

A value is what its type holds:

| Type | Value |
| --- | --- |
| `Float`, `Int`, `UInt` | A number. |
| `Vec2f` … `Vec4u` | A list of as many numbers, or a `vector` for two or three. |
| `Mat2f` … `Mat4f` | A list of every number, column by column. Columns are 16 bytes apart in a block (8 for a `Mat2f`) and end to end in a vertex, which the writer knows. |
| An array, `Light[4]` | A list, no longer than the array. |
| A struct | A table by field name; fields left out keep what they held. |

A mistake names the member: `@jess/render: pass.cell is 2 numbers, not 1`, `@jess/render: a Light has no field glow`, `@jess/render: the pass block has no member gamma`.

## The camera

{{ api_signature(value="require('@jess/render/camera'): { block, with, identity, multiply, perspective, orthographic, lookAt, write, transform }") }}

The standard camera is a convention a program opts into, not something the kernel knows. Its world is right-handed, a camera looks down its own -Z, and clip-space depth runs from 0 at the near plane to 1 at the far one, which is what every backend jess draws through expects. A matrix is 16 numbers, column by column.

| Member | Does |
| --- | --- |
| `camera.block` | The pass members a program declares to read the camera: `view`, `projection`, `viewProjection` (projection after view), and `position`, the camera's place in the world. |
| `camera.with(extra)` | `camera.block` and `extra` together, for a pass that declares more. |
| `camera.perspective(fovY, aspect, near, far)` | A perspective projection: `fovY` radians from the bottom of the view to its top, a width `aspect` times the height. |
| `camera.orthographic(left, right, bottom, top, near, far)` | A projection of that box in front of the camera. |
| `camera.lookAt(eye, target, up)` | The view from `eye` toward `target`, each a `vector`, `up` toward the top of the view. |
| `camera.multiply(after, before)` | The matrix that applies `before`, then `after`. |
| `camera.identity()` | The matrix that changes nothing. |
| `camera.write(block, view, projection, position)` | Sets the camera's four members of a block whose program declares `camera.block`. |
| `camera.transform(matrix, point)` | Where `matrix` takes `point`, divided by its `w`. |

```luau
--!strict
local camera = require '@jess/render/camera'
local render = require '@jess/render'
local shader = require '@jess/shader'

local program = shader.program {
  attributes = { position = shader.vec3 },
  pass = camera.with { exposure = shader.float },
  vertex = function(input)
    local world = shader.vec4(input.attributes.position, 1)

    return { position = input.pass.viewProjection * world }
  end,
  fragment = function(input) return shader.vec4(input.pass.exposure) end,
}

local eye = vector.create(0, 2, 5)
local view = camera.lookAt(eye, vector.zero, vector.create(0, 1, 0))
local projection = camera.perspective(math.rad(60), 16 / 9, 0.1, 100)

local pass = render.block(program.layout, 'pass')
camera.write(pass, view, projection, eye)
pass:set('exposure', 1.5)

local origin = camera.transform(camera.multiply(projection, view), vector.zero)
print(pass.size, string.format('%.3f', origin.z))
-- 224  0.982
```

`pass:upload(cameraBuffer)` then writes the block into a uniform buffer of `pass.size` bytes, which `pipeline:bind('pass', { [0] = cameraBuffer })` binds. The [windowed application](@/docs/scripting/applications.md#a-window) does exactly that every frame.
