---
title: 'jess.gpu'
description: 'The GPU in front of the script: the device jess opens on it, the buffers, textures, and samplers made on it, and the pipelines that draw and compute with them.'
weight: 47
extra:
  kind: api
---

{{ api_signature(value="jess.gpu: { device }") }}

`jess.gpu` is the GPU, as the kernel reaches it: an adapter, and the device opened on it. It says what the hardware is and does what it's told with bytes, images, and [shader programs](@/docs/api/shader.md), and has no opinion about what any of it means. What drawing means (cameras, materials, techniques) is `@jess/render`'s, in Luau, over this. `require('@jess/gpu')` is the same table.

A jess built without the `gpu` feature still has `jess.gpu`, and says so when asked for a device: `jess.gpu.device: this jess has no GPU: it was built without the gpu feature`.

## jess.gpu.device

{{ api_signature(value="jess.gpu.device() -> GpuDevice") }}

The runtime's device. The first call opens it, on whatever adapter the machine has: a hardware one if there is one, else a software one, which is what an offscreen render or a test on a machine without a GPU wants. Every later call is the same device.

- **Capability:** `gpu.device`. A script you run holds it, as it holds the input devices in front of you; a package target declares it. Without it: `jess.gpu.device: needs the 'gpu.device' capability, which this runtime does not grant`.
- **Errors:** `jess.gpu.device: no GPU adapter, not even a software one`, and an adapter that opens no device.

## GpuDevice

| Field | Type | Holds |
| --- | --- | --- |
| `name` | `string` | The adapter's name, as its driver gives it. |
| `backend` | `string` | The API wgpu reaches it through: `vulkan`, `metal`, `dx12`, or `gl`. |
| `kind` | `string` | `discrete`, `integrated`, `virtual`, `cpu` (a software adapter), or `other`. |
| `limits` | `{ textureSize, textureLayers, bufferSize, workgroups }` | The widest or tallest 2D texture, the most layers an array texture has, the largest buffer in bytes, and the most workgroups a dispatch has on one axis. |

`tostring(device)` is `GpuDevice(name, backend)`.

```luau
--!strict
local device = jess.gpu.device()
print(device.backend, device.kind)
print(device.limits.textureSize, device.limits.workgroups)
-- vulkan  integrated
-- 8192  65535
```

That's this machine's: a laptop's integrated GPU, reached through Vulkan. Yours will say what yours is.

## Resources

A device makes three kinds of resource. Each lives as long as a script holds it, and belongs to its runtime's device; nothing about one means anything to the GPU until a pipeline reads it.

### GpuDevice:buffer

{{ api_signature(value="device:buffer(size: number, usage: GpuBufferUsage | { GpuBufferUsage }) -> GpuBuffer") }}

`size` bytes, zeroed, a multiple of 4 up to `limits.bufferSize`. `usage` names what the buffer is for, one name or a list: `vertex`, `index`, `uniform`, or `storage`. A buffer can always be written and read back; `{}` makes one for nothing else.

### GpuDevice:texture

{{ api_signature(value="device:texture(desc: GpuTextureDesc) -> GpuTexture") }}

| Field | Type | Holds |
| --- | --- | --- |
| `width`, `height` | `number` | Its size in texels. |
| `format` | `GpuTextureFormat` | Every texel's format: `rgba8Unorm`, `rgba8UnormSrgb`, `bgra8Unorm`, `bgra8UnormSrgb`, `rgba16Float`, `r8Unorm`, `rg8Unorm`, `r16Float`, `rg16Float`, `r32Float`, `rg32Float`, `rgba32Float`, `r32Uint`, `rgba8Uint`, `rgba32Uint`, `r32Sint`, `rgba32Sint`, `depth24Plus`, or `depth32Float`. |
| `usage` | `GpuTextureUsage \| { GpuTextureUsage }` | `sampled` (shaders read it), `target` (passes draw into it), or both. |
| `shape` | `GpuTextureShape?` | `2d` (the default), `2dArray`, `cube` (six square faces, +x, -x, +y, -y, +z, -z), or `3d`. |
| `layers` | `number?` | A `2dArray`'s layers. |
| `depth` | `number?` | A `3d` texture's slices. |
| `mips` | `number?` | How many mip levels, the first full size and each after it half the last; 1 by default. |

A name the table doesn't take is an error listing the ones it does, so `layer = 2` doesn't quietly make a one-layer texture.

### GpuDevice:sampler

{{ api_signature(value="device:sampler(desc: GpuSamplerDesc?) -> GpuSampler") }}

| Field | Type | Holds |
| --- | --- | --- |
| `filter` | `('nearest' \| 'linear')?` | How it reads between texels; `nearest` by default. |
| `mipmapFilter` | `('nearest' \| 'linear')?` | How it reads between mip levels; `nearest` by default. |
| `address` | `('clamp' \| 'repeat' \| 'mirror')?` | What it reads past an edge, on every axis: the edge texel (the default), the texture tiled, or the texture mirrored. |
| `compare` | `GpuCompare?` | Makes a comparison sampler, which compares a reference with what it reads: `never`, `less`, `equal`, `lessEqual`, `greater`, `notEqual`, `greaterEqual`, or `always`. It reads depth textures, for shadows. |

## GpuBuffer

| Member | Type | Does |
| --- | --- | --- |
| `size` | `number` | Its size in bytes. |
| `usage` | `{ GpuBufferUsage }` | What it is for, in the order `vertex`, `index`, `uniform`, `storage`. |
| `write` | `(self, offset: number, bytes: buffer \| string, from: number?, length: number?) -> ()` | Copies `bytes` (or `length` of them from `from`) into the buffer at `offset`, before the next work the GPU is given. Luau's bytes are read in place, not copied first. |
| `read` | `(self, offset: number?, length: number?) -> buffer` | Waits for everything submitted so far, and returns `length` bytes from `offset` (by default, all of it) as a new `buffer`. |

Offsets and lengths are multiples of 4, and a range past the end is an error. `tostring(gpuBuffer)` is `GpuBuffer(16 bytes, vertex, storage)`.

```luau
--!strict
local device = jess.gpu.device()
local positions = device:buffer(16, { 'vertex', 'storage' })
local bytes = buffer.create(16)
buffer.writef32(bytes, 0, 1.5)
buffer.writef32(bytes, 12, -2)
positions:write(0, bytes)
positions:write(4, 'abcdefgh', 2, 4)

local back = positions:read()
print(positions, buffer.readf32(back, 0), buffer.readstring(back, 4, 4), buffer.readf32(back, 12))
-- GpuBuffer(16 bytes, vertex, storage)  1.5  cdef  -2
```

## GpuTexture

| Member | Type | Does |
| --- | --- | --- |
| `width`, `height` | `number` | Its size in texels. |
| `format` | `GpuTextureFormat` | Its texels' format. |
| `shape` | `GpuTextureShape` | `2d`, `2dArray`, `cube`, or `3d`. |
| `layers` | `number` | How many 2D layers it has: an array's layers, a cube's 6, 1 otherwise. |
| `mips` | `number` | How many mip levels. |
| `write` | `(self, bytes: buffer \| string, level: number?, layer: number?) -> ()` | Replaces every texel of mip `level` of `layer` (every slice, for a `3d` texture), rows packed tightly, top row first. Both default to 0. |
| `read` | `(self, level: number?, layer: number?) -> buffer` | Waits for everything submitted so far, and returns that level of that layer the way `write` takes it. |

The bytes are exactly the level's: width × height × the format's texel size. A pass writes a depth texture; a script can't, and `depth24Plus`, whose layout is the backend's, can't be read back either. `tostring(texture)` is `GpuTexture(2×2 rgba8Unorm, 2dArray)`.

```luau
--!strict
local device = jess.gpu.device()
local atlas = device:texture({
  width = 2,
  height = 2,
  format = 'rgba8Unorm',
  usage = 'sampled',
  shape = '2dArray',
  layers = 4,
  mips = 2,
})
atlas:write(string.rep('\255\0\0\255', 4), 0, 3)
atlas:write('\0\0\255\255', 1, 3)

local level = atlas:read(1, 3)
print(atlas, atlas.layers, atlas.mips, buffer.len(level), buffer.readu8(level, 2))
-- GpuTexture(2×2 rgba8Unorm, 2dArray)  4  2  4  255
```

## GpuSampler

A sampler has nothing to read: it is what a draw binds beside a texture. `tostring(sampler)` is `GpuSampler(linear)`, or `GpuSampler(nearest, comparing)` for a comparison sampler.

## Pipelines

### GpuDevice:pipeline

{{ api_signature(value="device:pipeline(program: ShaderProgram, state: GpuPipelineState?) -> GpuPipeline") }}

A [shader program](@/docs/api/shader.md), lowered, validated by the backend, and made ready to run. A compute program takes no state. A vertex and fragment program takes the fixed-function state everything else about drawing needs:

| Field | Type | Holds |
| --- | --- | --- |
| `colors` | `{ GpuTextureFormat \| { format, blend } }` | A color target per fragment output, by location, each of a format the output fits. `blend` is `replace` (the default), `alpha`, `premultiplied`, or `additive`; an integer target only replaces. |
| `depth` | `(GpuTextureFormat \| { format, compare, write })?` | The depth target it tests against. A format's name alone tests `less` and writes; `compare` is a `GpuCompare` and `write` a boolean. |
| `topology` | `string?` | `triangles` (the default), `triangleStrip`, `lines`, `lineStrip`, or `points`. |
| `cull` | `string?` | Which triangles it throws away: `none` (the default), `front`, or `back`. |
| `clockwise` | `boolean?` | Whether a triangle running clockwise faces the camera; by default a counter-clockwise one does. |

The program's own layout decides everything else: its bind groups, its vertex buffers' strides and attributes. A pipeline can't disagree with its program.

## GpuPipeline

| Member | Type | Does |
| --- | --- | --- |
| `kind` | `'render' \| 'compute'` | Whether it draws or computes. |
| `bind` | `(self, frequency, bindings) -> GpuBindGroup` | What one frequency binds for it. |

`tostring(pipeline)` is `GpuPipeline(render, 1 color target, depth)`, or `GpuPipeline(compute 64×1×1)`.

### GpuPipeline:bind

{{ api_signature(value="pipeline:bind(frequency: 'pass' | 'material' | 'draw', bindings: { [number]: GpuBuffer | GpuTexture | GpuSampler }) -> GpuBindGroup") }}

`bindings` are keyed by binding number, the numbers [program.layout](@/docs/api/shader.md#layout) reports: the uniform block at 0, if the frequency has members, then each texture, sampler, and storage buffer. Every declaration is bound exactly once, with a resource of its kind: a uniform buffer at least the block's size, a texture of the declared shape and sample kind, a comparison sampler where one is declared, a storage buffer. A number the program doesn't bind is an error listing the ones it does: `the program binds nothing at material binding 5: it binds 0 (the uniform block), 1 (the texture diffuse), 2 (diffuse's sampler)`.

Names belong to whatever builds on this: the kernel takes numbers, so a Luau library can bind a material by its declarations' names without the kernel deciding what a name means.

## GpuBindGroup

What one frequency binds. It binds for every pipeline whose program lays that frequency out the same way (a uniform block of the same size, and the same kinds of texture, sampler, and storage buffer at the same binding numbers), whichever pipeline it was made for, so one camera block serves every program that declares it alike. `frequency` is `pass`, `material`, or `draw`, and `tostring(group)` is `GpuBindGroup(material)`.

## Passes

### GpuDevice:render

{{ api_signature(value="device:render(pass: GpuPass)") }}

Draws `draws`, in order, into `colors` (by location) and `depth`, and submits the pass. A target is a texture made with the `target` usage, and keeps what it holds, or a table:

| Field | Type | Holds |
| --- | --- | --- |
| `texture` | `GpuTexture` | The texture drawn into. |
| `clear` | `({ number } \| number)?` | What it is cleared to first: four channels (whole numbers for an integer format), or a depth from 0 to 1. Without one, the pass draws over what's there. |
| `level`, `layer` | `number?` | The mip level, and the array layer or cube face, drawn into; 0 by default. |
| `store` | `boolean?` | `false` throws away what the pass drew. |

A draw is a table:

| Field | Type | Holds |
| --- | --- | --- |
| `pipeline` | `GpuPipeline` | A render pipeline whose targets are the pass's. |
| `count` | `number` | How many vertices, or with `indices`, how many indices. |
| `instanceCount` | `number?` | How many instances; 1 by default. |
| `pass`, `material`, `draw` | `GpuBindGroup?` | A bind group for each frequency its program binds anything for, laid out as its program lays that frequency out. |
| `vertices`, `instances` | `GpuBuffer?` | The per-vertex and per-instance attributes, if the program has any, in vertex buffers holding every one drawn. |
| `indices`, `indexFormat` | `GpuBuffer?`, `('u16' \| 'u32')?` | An index buffer, and how wide its indices are (`u32` by default). |

Everything is checked before anything is submitted, and each mistake names its draw: `draw 0 is invalid: its program binds a material group, and the draw gives none`. A pass reads its draws one at a time, so a pass of ten thousand draws holds no more of the Luau stack than a pass of one.

```luau
--!strict
local shader = require '@jess/shader'

local program = shader.program {
  attributes = { position = shader.vec2 },
  material = { tint = shader.vec4 },
  vertex = function(input)
    return { position = shader.vec4(input.attributes.position, 0, 1) }
  end,
  fragment = function(input) return input.material.tint end,
}

local device = jess.gpu.device()
local pipeline = device:pipeline(program, { colors = { 'rgba8Unorm' } })

local tint = device:buffer(program.layout.material.size or 0, 'uniform')
local orange = buffer.create(16)
for index, channel in { 1, 0.5, 0, 1 } do
  buffer.writef32(orange, (index - 1) * 4, channel)
end
tint:write(0, orange)

-- One triangle that covers the whole target.
local corners = buffer.create(24)
for index, value in { -1, -1, 3, -1, -1, 3 } do
  buffer.writef32(corners, (index - 1) * 4, value)
end
local vertices = device:buffer(24, 'vertex')
vertices:write(0, corners)

local image = device:texture({ width = 2, height = 2, format = 'rgba8Unorm', usage = 'target' })
device:render({
  colors = { { texture = image, clear = { 0, 0, 0, 1 } } },
  draws = {
    {
      pipeline = pipeline,
      material = pipeline:bind('material', { [0] = tint }),
      vertices = vertices,
      count = 3,
    },
  },
})

local pixels = image:read()
print(pipeline, buffer.readu8(pixels, 0), buffer.readu8(pixels, 1), buffer.readu8(pixels, 2))
-- GpuPipeline(render, 1 color target)  255  128  0
```

### GpuDevice:dispatch

{{ api_signature(value="device:dispatch(work: GpuDispatch)") }}

Runs a compute pipeline over `workgroups` workgroups (a count, or one to three of them, one per axis, the rest 1), each of the program's workgroup size, with a bind group for each frequency its program binds, and submits it. A buffer read afterwards waits for it.

```luau
--!strict
local shader = require '@jess/shader'

local double = shader.compute {
  workgroupSize = { 64 },
  pass = { values = shader.storage(shader.float, 'readWrite') },
  compute = function(input)
    local index, values = input.builtin.globalInvocationId.x, input.pass.values
    shader.when(shader.less(index, shader.arrayLength(values)), function()
      shader.write(values, index, values[index] * 2)
    end)
  end,
}

local device = jess.gpu.device()
local pipeline = device:pipeline(double)
local values = device:buffer(16, 'storage')
values:write(0, buffer.fromstring(string.pack('<ffff', 1, 2, 3, 4)))

device:dispatch({
  pipeline = pipeline,
  pass = pipeline:bind('pass', { [0] = values }),
  workgroups = 1,
})
print(string.unpack('<ffff', buffer.tostring(values:read())))
-- 2  4  6  8  17
```

The last number is `string.unpack`'s: where it stopped reading.
