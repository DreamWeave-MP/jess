---
title: 'jess.gpu'
description: 'The GPU in front of the script: the device jess opens on it, the buffers, textures, and samplers made on it, the pipelines that draw and compute with them, the draw sets kept on it, the plans that run a frame, and the windows that show one.'
weight: 47
extra:
  kind: api
---

{{ api_signature(value="jess.gpu: { device, window, pump }") }}

`jess.gpu` is the GPU, as the kernel reaches it: an adapter, and the device opened on it. It says what the hardware is and does what it's told with bytes, images, and [shader programs](@/docs/api/shader.md), and has no opinion about what any of it means. What drawing means (cameras, materials, techniques) is [`@jess/render`](@/docs/api/render.md)'s, in Luau, over this. `require('@jess/gpu')` is the same table.

A release has the GPU and windows, and loads the machine's GPU API and window system when a script first asks for them, so a machine without either still runs jess. A jess built without the `gpu` feature (`--no-default-features`) still has `jess.gpu`, and says so when asked for a device: `jess.gpu.device: this jess has no GPU: it was built without the gpu feature`. [GPU](@/docs/contributing/gpu.md) is how it works inside, and [Troubleshooting](@/docs/reference/troubleshooting.md#gpus-windows-and-sound) has what to do when a machine has no GPU or no display.

## jess.gpu.device

{{ api_signature(value="jess.gpu.device() -> GpuDevice") }}

The runtime's device. The first call asks the backend for a low-power adapter and, if that fails, tries a fallback adapter. The backend and machine determine which adapter is available; a fallback may be a software adapter. Every later call is the same device.

- **Capability:** `gpu.device`. A script you run holds it, as it holds the input devices in front of you; a package target declares it ([Capabilities](@/docs/reference/capabilities.md) has the rest). Without it: `jess.gpu.device: needs the 'gpu.device' capability, which this runtime does not grant`.
- **Generations:** in an application, the device is the application's: the first generation to ask opens it, and every later one gets the same device, so a restart reloads code without opening the GPU again. Everything a generation made on it (buffers, textures, pipelines, draw sets, plans, windows) goes with that generation's runtime, and no handle of one generation can reach the next.
- **Errors:** `jess.gpu.device: no GPU adapter, not even a software one`, and an adapter that opens no device, `jess.gpu.device: the GPU adapter opened no device: …` with the driver's reason.

## GpuDevice

| Field | Type | Holds |
| --- | --- | --- |
| `name` | `string` | The adapter's name, as its driver gives it. |
| `backend` | `string` | The API wgpu reaches it through: `vulkan`, `metal`, `dx12`, or `gl`. |
| `kind` | `string` | `discrete`, `integrated`, `virtual`, `cpu` (a software adapter), or `other`. |
| `limits` | `{ textureSize, textureLayers, bufferSize, workgroups }` | The widest or tallest 2D texture, the most layers an array texture has, the largest buffer in bytes, and the most workgroups a dispatch has on one axis. |
| `pipelines` | `{ compiled, reused }` | How many pipelines the device compiled, and how many times `GpuDevice:pipeline` gave back one it already had. |
| `timestamps` | `boolean` | Whether the device times passes, which `GpuPlan:timings` reports. |
| `live` | `{ buffers, textures, samplers, bindGroups, pipelines }` | How many of each the device has alive: made, and not yet let go of. A count that only grows is a leak. |
| `made` | `{ buffers, textures, samplers, bindGroups, pipelines }` | How many of each the device ever made, let go of or not. A count that grows every frame of a scene that doesn't change is churn: a run of a plan makes nothing the run before it made. |
| `sets` | `GpuDrawSets` | The device's own [draw sets](#draw-sets), which a draw set joins unless told otherwise and a run draws from unless given others. |

`tostring(device)` is `GpuDevice(name, backend)`.

```luau
--!strict
local device = jess.gpu.device()
print(device.backend, device.kind)
print(device.limits.textureSize, device.limits.workgroups)
-- vulkan  integrated
-- 8192  65535
```

Those numbers are one integrated GPU's, reached through Vulkan. `kind` says what yours is, and `cpu` means a software adapter. Available limits and supported operations depend on the adapter; check the destination rather than assuming that a software adapter supports every workload.

## Commits

What a script changes about what the GPU draws (a buffer or texture it writes, a draw set's counts, whether it is visible, which collection holds it, which passes a plan leaves out, what a copy pass copies) waits for the device's next commit, and the commit applies all of it at once. Whatever draws a frame on its own, a window, draws a commit's changes whole or none of them, never half a frame.

A commit happens at the end of every frame under an application's host, at `jess.gpu.pump()`, at `device:commit()`, and before anything that uses the GPU and waits for it (`plan:run`, `device:render`, `device:dispatch`, a read), so a script always sees its own changes when it asks the GPU for something. What reads a draw set or a plan back (`set.count`, `set.added`, `sets:has`, `plan:enabled`) says what the last commit left. Mistakes are refused when the change is asked for, not at the commit. A commit is whole in what a frame sees, but it isn't a transaction: a change the GPU itself refuses at the commit is reported, the changes after it still apply, and nothing before it is rolled back.

| Member | Type | Does |
| --- | --- | --- |
| `commit()` | `() -> boolean` | Applies everything changed since the last commit, saying whether there was anything. |
| `room` | `{ changes, bytes }` | How much more fits before the next commit: 65 536 changes carrying 256 MiB of data a frame. A change that doesn't fit is refused, naming what waits; commit, or change less. |

## Resources

A device makes three kinds of resource. Each lives as long as a script holds it, and belongs to its runtime's device; nothing about one means anything to the GPU until a pipeline reads it.

### GpuDevice:buffer

{{ api_signature(value="device:buffer(size: number, usage: GpuBufferUsage | { GpuBufferUsage }) -> GpuBuffer") }}

`size` bytes, zeroed, a multiple of 4 from 4 through `limits.bufferSize`. Zero, an unaligned size, or a size over the device limit raises an error. `usage` names what the buffer is for, one name or a list: `vertex`, `index`, `uniform`, `storage`, or `indirect` (the counts of an indirect draw). A buffer can always be written and read back; `{}` makes one for nothing else.

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
| `samples` | `number?` | Samples a texel: 1 (the default), or 4 for a multisampled `2d` `target` of one level, which a pass draws into and [resolves](#gpudevice-render). Nothing samples, writes, or reads one back; only formats the device multisamples take 4. |

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
| `usage` | `{ GpuBufferUsage }` | What it is for, in the order `vertex`, `index`, `uniform`, `storage`, `indirect`. |
| `write` | `(self, offset: number, bytes: buffer \| string, from: number?, length: number?) -> ()` | Copies `bytes` (or `length` of them from `from`) into the buffer at `offset`, at the next [commit](#commits). |
| `read` | `(self, offset: number?, length: number?) -> buffer` | Commits pending changes and returns `length` bytes from `offset` as a new `buffer`, waiting for submitted work when the length is nonzero. The offset defaults to zero and the length to the remainder. |
| `readLater` | `(self, offset: number?, length: number?) -> GpuRead` | Commits pending changes and submits a copy without waiting for GPU completion; the [read](#gpuread) supplies the bytes when ready. |

Offsets and lengths are whole numbers from 0 through 4,294,967,295. GPU offsets and transferred lengths must also be multiples of 4, and a range past the end is an error. The source `from` offset for `write` is zero-based, defaults to zero, and can be unaligned as long as the resulting transferred length is aligned; its `length` defaults to the remaining source bytes, capped at 4,294,967,295. `read` and `readLater` apply the same cap to their default length. An unaligned default is rejected, so specify an aligned length when the remaining range exceeds that cap. `tostring(gpuBuffer)` is `GpuBuffer(16 bytes, vertex, storage)`.

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
| `write` | `(self, bytes: buffer \| string, level: number?, layer: number?) -> ()` | Replaces every texel of mip `level` of `layer` (every slice, for a `3d` texture), rows packed tightly, top row first, at the next [commit](#commits). Both default to 0. |
| `read` | `(self, level: number?, layer: number?) -> buffer` | Waits for everything submitted so far, and returns that level of that layer the way `write` takes it. |
| `readLater` | `(self, level: number?, layer: number?) -> GpuRead` | Commits pending changes and submits a copy without waiting for GPU completion: a [read](#gpuread). |

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

## GpuRead

Bytes on their way back from the GPU, from `readLater`. They are what the submitted copy saw, so a script that wants a frame's picking id can ask after the run and poll for it later without waiting for GPU completion. Completion time depends on the device and workload.

| Member | Type | Does |
| --- | --- | --- |
| `poll()` | `() -> buffer?` | The bytes, once the GPU has copied them; `nil` until then. It polls the device without waiting for GPU completion and copies the mapped bytes when ready. Device or mapping failures raise. |
| `wait()` | `() -> buffer` | The bytes, waiting for submitted GPU work if needed. Device or mapping failures raise. |
| `ready` | `boolean` | Whether the last `poll` or `wait` found them. |

Once there, the same bytes come back every time. `tostring(read)` is `GpuRead(waiting)` or `GpuRead(ready)`.

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
| `multisampled` | `boolean?` | Whether it draws into targets of 4 samples a texel; single-sample ones by default. A pass's targets and its pipelines agree on it. |

The program's own layout decides everything else: its bind groups, its vertex buffers' strides and attributes. A pipeline can't disagree with its program.

The device compiles a pipeline once. Asking again for a program that lowers to the same shader, with the same state, gives back the pipeline it already has, for as long as anything holds it; `device.pipelines` counts how often it compiled and how often it reused one. Writing a material's buffer changes what a pipeline draws, not the pipeline, so it compiles nothing.

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

{{ api_signature(value="device:render(pass: GpuPass) -> ()") }}

Draws `draws`, in order, into `colors` (by location) and `depth`, and submits the pass. A target is a texture made with the `target` usage, and keeps what it holds, or a table:

| Field | Type | Holds |
| --- | --- | --- |
| `texture` | `GpuTexture` | The texture drawn into. |
| `clear` | `({ number } \| number)?` | What it is cleared to first: four channels (whole numbers for an integer format), or a depth from 0 to 1. Without one, the pass draws over what's there. |
| `level`, `layer` | `number?` | The mip level, and the array layer or cube face, drawn into; 0 by default. |
| `store` | `boolean?` | `false` throws away what the pass drew. |
| `resolve` | `(GpuTexture \| { texture, level, layer })?` | Where a multisampled color target's samples average into when the pass ends: level 0 of layer 0 of a single-sample `target` texture of its format, or the level and layer named, of its size. The samples themselves can then go unstored. |

A draw is a table:

| Field | Type | Holds |
| --- | --- | --- |
| `pipeline` | `GpuPipeline` | A render pipeline whose targets are the pass's. |
| `count` | `number` | How many vertices, or with `indices`, how many indices. |
| `instanceCount` | `number?` | How many instances; 1 by default. |
| `pass`, `material`, `draw` | `GpuBindGroup?` | A bind group for each frequency its program binds anything for, laid out as its program lays that frequency out. |
| `vertices`, `instances` | `GpuBuffer?` | The per-vertex and per-instance attributes, if the program has any, in vertex buffers holding every one drawn. |
| `indices`, `indexFormat` | `GpuBuffer?`, `('u16' \| 'u32')?` | An index buffer, and how wide its indices are (`u32` by default). |
| `indirect`, `indirectOffset` | `GpuBuffer?`, `number?` | Where the counts come from instead of `count` and `instanceCount`: an `indirect` buffer holding, from `indirectOffset` (0 by default, a multiple of 4), four `u32`s (vertex count, instance count, first vertex, first instance) or, for an indexed draw, five (index count, instance count, first index, base vertex, first instance). Whatever the GPU wrote there is drawn, a compute pass's count of what it kept, say; nothing checks it against the buffers, and the GPU keeps a draw past their ends inside them. |

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

{{ api_signature(value="device:dispatch(work: GpuDispatch) -> ()") }}

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

## Draw sets

### GpuDevice:drawSet

{{ api_signature(value="device:drawSet(desc: GpuDrawSetDesc) -> GpuDrawSet") }}

Keeps a draw in a collection of draw sets, the device's own (`device.sets`) unless `sets` names another: a render pipeline, its `material` and `draw` bind groups, its `vertices`, `instances`, and `indices` (with `indexFormat`), `count` and `instanceCount` (or the `indirect` buffer the GPU writes them into), and the `tags` passes choose it by. The fields are a draw's, as `GpuDevice:render` takes them, without `pass`: a draw set is drawn with the pass group of whichever pass takes it, so one camera block serves every set a pass draws. It is checked when it is made, the way a pass checks a draw, and drawn from the next frame on by every draw pass whose tags it carries, until it is removed.

## GpuDrawSet

| Member | Type | Holds |
| --- | --- | --- |
| `count` | `number` | How many vertices, or indices, it draws. Setting it checks it fits the buffers. |
| `instanceCount` | `number` | How many instances; at least 1, and no more than its instance buffer holds. |
| `visible` | `boolean` | Whether passes draw it. |
| `tags` | `{ string }` | What the passes that take it look for. |
| `added` | `boolean` | Whether it is in the collection it was made into. |
| `add()`, `remove()` | `() -> ()`, `() -> boolean` | Puts it back in that collection, or takes it out, saying whether it was there. Both are harmless twice. |

Changing a set changes nothing else: what it draws is read again at every frame, so nothing is rebuilt.

### GpuDevice:drawSets

{{ api_signature(value="device:drawSets() -> GpuDrawSets") }}

A collection of its own: the draw sets of one scene, say, which a run draws from when it is given them (`plan:run(imports, sets)`, `window:show(plan, target, imports, sets)`) instead of the device's. A set can be in any number of collections. A plan remembers which sets carry its draw passes' tags, for each collection, until something is added to or removed from it; showing and hiding a set, or changing its counts, needs no choosing again.

| Member | Type | Does |
| --- | --- | --- |
| `count` | `number` | How many sets it holds. |
| `add(set)` | `(GpuDrawSet) -> ()` | Adds a set, last; adding it again does nothing. |
| `remove(set)` | `(GpuDrawSet) -> boolean` | Takes a set out, saying whether it was there. |
| `has(set)` | `(GpuDrawSet) -> boolean` | Whether it holds a set. |

`tostring(sets)` is `GpuDrawSets(3 sets)`.

```luau
--!strict
local shader = require '@jess/shader'

local program = shader.program {
  attributes = { position = shader.vec2 },
  pass = { tint = shader.vec4 },
  vertex = function(input)
    return { position = shader.vec4(input.attributes.position, 0, 1) }
  end,
  fragment = function(input) return input.pass.tint end,
}

local device = jess.gpu.device()
local pipeline = device:pipeline(program, { colors = { 'rgba8Unorm' } })
local quad = device:drawSet {
  pipeline = pipeline,
  vertices = device:buffer(48, 'vertex'),
  count = 6,
  tags = { 'opaque' },
}

device:commit()
quad.count = 3
quad.visible = false
print(quad.count, quad:remove())
device:commit()
print(quad, quad.added)
-- 6  true
-- GpuDrawSet(3 vertices, opaque, hidden)  false
```

## Plans

### GpuDevice:plan

{{ api_signature(value="device:plan(desc: GpuPlanDesc) -> GpuPlan") }}

Compiles a [render graph](@/docs/contributing/render-graph.md) for the device: the targets and buffers a frame uses, the passes that use them, and what each pass does. Passes are ordered by what they read and write, not by where they are in the list; passes whose work nothing sees are culled; transient targets and buffers whose lifetimes don't overlap share memory. Planning allocates them and makes every bind group it can, so a frame does nothing but record.

| Field | Type | Holds |
| --- | --- | --- |
| `name` | `string?` | The plan's name, `plan` by default, and every pass's owner unless the pass names one. |
| `viewport` | `{ width, height }` | The pixels relative targets are sized against. |
| `targets` | `{ GpuPlanTarget }?` | `{ name, format, scale \| width and height, mips, layers, samples, imported, kept }`: a color or depth target the viewport's size times `scale` (1 by default), or a fixed size, of `mips` levels and `layers` layers (1 of each by default) and `samples` samples a texel (1, or 4 for a multisampled target of one level and one layer). An `imported` one is given to each run, a window's backbuffer, say; a `kept` one is the plan's from run to run (below). |
| `buffers` | `{ GpuPlanBuffer }?` | `{ name, size \| image and scale, imported, kept }`: `size` bytes, a multiple of 4, or room for all of an `image` target of that format at the viewport's size times `scale` (1 by default), each row padded as a copy lays it out, so a copy of a whole target fits it at any viewport. |
| `passes` | `{ GpuPlanPass }` | What runs: below. |

Every pass has a `name` and a `kind`, and may name an `owner`: whoever declared it, a package or a subsystem, for profilers and graph inspectors to attribute it to. It is the plan's `name` unless the pass says otherwise, and never empty. What a pass reads and writes depends on the kind:

| Kind | Does | Fields |
| --- | --- | --- |
| `draw` | Draws the device's visible draw sets carrying any of `tags`, in the order they were made, each with `pass` as its pass group. | `tags`, `pass`, `pipeline` (the one whose layout a `pass` of bindings is made for), `colors`, `depth`, `depthTest`, `reads` |
| `fullscreen` | Draws three vertices of `pipeline`: one triangle over its targets. | `pipeline`, `pass`, `material`, `draw`, `colors`, `depth`, `depthTest`, `reads` |
| `compute` | Dispatches `pipeline` over `workgroups` (1 by default). | `pipeline`, `pass`, `material`, `draw`, `storage`, `workgroups`, `reads` |
| `copy` | Copies `from` into `to`; `load` keeps what the copy doesn't cover. | `from`, `to`, `size`, `load` |

- `colors` and `depth` are the targets drawn into: a name, or `{ target, clear, load, store, level, layer, resolve }`, cleared first, loaded from what it held, or neither, and stored unless `store` is `false`. A write draws into level 0 of layer 0 unless it names another. The first pass to write a target can't load it: nothing is there yet. A multisampled color target's samples average into its `resolve` when the pass ends, a target's name or `{ target, level, layer }`, single-sample, of its format, and the same size.
- `depthTest` names a depth target the pass tests against without writing it, or `{ target, level, layer }` for one level of one layer. A draw set whose pipeline writes depth isn't drawn in such a pass: the run refuses it.
- `reads` names what the pass reads: targets as textures, and buffers its shaders read as storage or its draws take vertices, instances, or indirect counts from. A draw set's buffers aren't the graph's, so a draw pass names the imported buffers its sets draw from here, and the passes that write them run first. A target read by name is read whole, bound as a `texture2dArray` where it has layers; `{ target, level, layer }` reads one level of one layer, bound as a `texture2d`. A multisampled target is only drawn into and resolved, never read.
- Each level of each layer is a target of its own as far as order goes: a pass that reads level 0 and draws into level 1 of the same target is a step of a mip chain, not a conflict, and a read of a whole target waits for the last writer of every level of every layer, all of which some pass has to write.
- `storage` names the buffers a compute pass writes: a name, or `{ buffer, clear, load, store }`, where `clear = true` zeroes it first, at every run.
- A group (`pass`, `material`, `draw`) is a `GpuBindGroup`, or bindings by binding number, as `GpuPipeline:bind` takes them, where a binding can also be a target's or a buffer's name. A texture binding names a target the pass `reads`; a storage binding names a buffer the pass reads, for a `'read'` declaration, or writes in `storage`, for a `'readWrite'` one.
- A `kept` target or buffer is the plan's own, and it shares no memory. It holds what the last run left (zeros before the first, and after a resize), so a pass can load it, as a frame's history, say. It counts as something the plan produces, like an imported one, and the script reads it with `plan:read` and `plan:readLater`, sized as the plan's viewport makes it: a readback of the whole frame without a buffer to manage.
- A copy's `from` and `to` are names, `{ target, level, layer, column, row }` for a level of a layer of a target from a texel, or `{ buffer, offset }` for a buffer from a byte, a multiple of 4. It copies everything from where `from` starts, or `size`: `{ width, height }` texels, or a number of bytes between buffers. A depth target is copied a whole level of a layer at a time.
- A copy between a target and a buffer lays out each row padded to a multiple of 256 bytes, the way the GPU copies, and the buffer has to hold every row from its offset.
- A relative target with more than one level never resolves smaller than its levels need: half the viewport with 5 levels is at least 16 pixels each way, so at a 10×600 viewport it is 16×300. A pass draws into relative targets of one scale and one number of levels together, which stay one size at every viewport.

Mistakes in the description name the pass: `GpuDevice:plan: pass present uses nowhere, which the plan has no target or buffer called`, `pass "q" both reads and writes "output"`.

## GpuPlan

| Member | Type | Holds |
| --- | --- | --- |
| `name` | `string` | The plan's name. |
| `order` | `{ string }` | The passes that run, in the order they run. |
| `viewport` | `{ width, height }` | What its relative targets are sized for. |
| `allocations` | `number` | How many textures and buffers it made for its own targets and buffers: one for each group sharing memory. |
| `run(imports, sets)` | `({ [string]: GpuTexture \| GpuBuffer }?, GpuDrawSets?) -> ()` | Runs every pass it records once, in one submission, with a texture or buffer for each imported target or buffer by name (a texture of its format and size at the viewport, a buffer of its size), its draw passes drawing from `sets`, or the device's own. |
| `setEnabled(passes, enabled)` | `(string \| { string }, boolean) -> ()` | Leaves passes out of the runs after it, or puts them back, together. A pass left out does nothing, so whatever it stores keeps what it held: a pass still recorded can't read or load it (`pass present uses what pass overlay stores: leave both out, or neither`), and a culled pass is refused, since it never runs. |
| `enabled(pass)` | `(string) -> boolean` | Whether runs record a pass. |
| `read(name, level, layer)` | `(string, number?, number?) -> buffer` | What a kept buffer, or `level` of `layer` of a kept target, holds once the GPU has finished the runs before the call, the way `GpuBuffer:read` and `GpuTexture:read` read them. |
| `readLater(name, level, layer)` | `(string, number?, number?) -> GpuRead` | The same, [without waiting](#gpuread). |
| `bytes(buffer)` | `(string) -> number` | How many bytes a buffer holds at the plan's viewport: what an imported one sized by it must be given. |
| `extent(target)` | `(string) -> { width, height }` | How big level 0 of a target is at the plan's viewport. |
| `setRegion(pass, region)` | `(string, GpuCopyRegion) -> ()` | Changes what a copy pass copies from the next run on: `{ from, to, size }`, where `from` and `to` are `{ column, row }` in a target or `{ offset }` in a buffer, of the level and layer the pass names. It is checked now, and resolved again at each size the plan is resized to. |
| `resized(width, height)` | `(number, number) -> GpuPlan` | The same plan for another viewport, its targets made again at the new size. It shares the passes left out and the regions with the plan it was resized from, both ways, as the plan a window shows does. |
| `dump()` | `() -> string` | The compiled graph: the passes in order, where each was declared (the `device:plan` call: a pass is a table, with no place of its own), what each reads and writes, what was culled, every target's lifetime, and what shares memory. |
| `timings()` | `() -> { { pass, ms } }?` | How long each pass of the last run took on the GPU, in milliseconds, in the order they ran; a copy pass has no `ms`. `nil` on a device without `timestamps`. Asking waits for that run to finish, so ask once in a while, not every frame. |

`tostring(plan)` is `GpuPlan(name, 4 of 5 passes, 640×480)`.

```luau
--!strict
local shader = require '@jess/shader'

local device = jess.gpu.device()
local flat = shader.program {
  attributes = { position = shader.vec2 },
  pass = { tint = shader.vec4 },
  vertex = function(input)
    return { position = shader.vec4(input.attributes.position, 0, 1) }
  end,
  fragment = function(input) return input.pass.tint end,
}
local pipeline = device:pipeline(flat, { colors = { 'rgba8Unorm' } })

local corners = buffer.create(24)
for index, value in { -1, -1, 3, -1, -1, 3 } do
  buffer.writef32(corners, (index - 1) * 4, value)
end
local vertices = device:buffer(24, 'vertex')
vertices:write(0, corners)
device:drawSet { pipeline = pipeline, vertices = vertices, count = 3, tags = { 'scene' } }

local orange = device:buffer(16, 'uniform')
orange:write(0, buffer.fromstring(string.pack('<ffff', 1, 0.5, 0, 1)))

local plan = device:plan {
  name = 'tiny',
  viewport = { width = 4, height = 4 },
  targets = { { name = 'screen', format = 'rgba8Unorm', imported = true } },
  buffers = { { name = 'pixels', size = 256 * 4, imported = true } },
  passes = {
    {
      name = 'scene',
      kind = 'draw',
      tags = { 'scene' },
      pass = pipeline:bind('pass', { [0] = orange }),
      colors = { { target = 'screen', clear = { 0, 0, 0, 1 } } },
    },
    { name = 'readBack', kind = 'copy', from = 'screen', to = 'pixels' },
  },
}

local screen = device:texture { width = 4, height = 4, format = 'rgba8Unorm', usage = 'target' }
local pixels = device:buffer(256 * 4, {})
plan:run { screen = screen, pixels = pixels }
print(plan, table.concat(plan.order, ', '))
print(string.unpack('BBBB', buffer.tostring(pixels:read(0, 4))))
-- GpuPlan(tiny, 2 of 2 passes, 4×4)  scene, readBack
-- 255  128  0  255  5
```

The last number is `string.unpack`'s, again. A run draws the draw sets as they are when it runs: change one, and the next run draws it changed, with nothing planned again.

A mip chain lives in one target: each pass reads one level and draws the next, and a copy takes a corner of the last level.

```luau
--!strict
local shader = require '@jess/shader'

local device = jess.gpu.device()
local function covering(input: ShaderInput): { [string]: ShaderExpr }
  local index = input.builtin.vertexIndex
  local across = shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2))
  local down = shader.bitAnd(index, shader.uint(2))
  local corner = shader.vec2(shader.float(across), shader.float(down))
  return { position = shader.vec4(corner * 2 - 1, 0, 1) }
end

local fill = device:pipeline(shader.program {
  vertex = covering,
  fragment = function() return shader.vec4(1, 0.5, 0, 1) end,
}, { colors = { 'rgba8Unorm' } })

local halve = device:pipeline(shader.program {
  material = { above = shader.texture2d },
  vertex = covering,
  fragment = function(input)
    local at = shader.vec2u(input.builtin.position.xy * 2)
    return shader.load(input.material.above, at, 0) * 0.5
  end,
}, { colors = { 'rgba8Unorm' } })

local plan = device:plan {
  name = 'chain',
  viewport = { width = 1, height = 1 },
  targets = { { name = 'chain', format = 'rgba8Unorm', width = 8, height = 8, mips = 3 } },
  buffers = { { name = 'pixels', size = 256 * 2, imported = true } },
  passes = {
    { name = 'fill', kind = 'fullscreen', pipeline = fill, colors = { 'chain' } },
    {
      name = 'down',
      kind = 'fullscreen',
      pipeline = halve,
      material = { [0] = 'chain', [1] = device:sampler() },
      reads = { { target = 'chain', level = 0 } },
      colors = { { target = 'chain', level = 1 } },
    },
    {
      name = 'corner',
      kind = 'copy',
      from = { target = 'chain', level = 1, column = 2, row = 2 },
      to = 'pixels',
      size = { width = 2, height = 2 },
    },
  },
}

local pixels = device:buffer(256 * 2, {})
plan:run { pixels = pixels }
print(string.unpack('BBBB', buffer.tostring(pixels:read(0, 4))))
-- 128  64  0  128  5
```

## Windows

{{ api_signature(value="jess.gpu.window(device: GpuDevice, desc: GpuWindowDesc?) -> GpuWindow") }}

Opens a native window with a surface on `device`. The window is what a person sees an application in; which windows an application has, what they show, and when they close are the application's to decide, from Luau, with what follows. [A window](@/docs/scripting/applications.md#a-window) in the applications guide puts all of it together: a package whose application draws a triangle from a vertex buffer into a window, turns it every frame, and answers the window's resizes and its close.

| Field | Type | Holds |
| --- | --- | --- |
| `title` | `string?` | Its title; `jess` by default. |
| `width`, `height` | `number?` | Its inner size in physical pixels; 800 by 600 by default. The window system may give it another. |
| `resizable` | `boolean?` | Whether the person can resize it; `true` by default. |
| `vsync` | `boolean?` | Whether presenting waits for the display's refresh, one image a refresh; `true` by default. |
| `format` | `GpuTextureFormat?` | The format its surface draws in; by default the first of `bgra8Unorm`, `rgba8Unorm`, `bgra8UnormSrgb`, and `rgba8UnormSrgb` it offers. |
| `key` | `string?` | What the application calls it, to keep it: a window opened with a key stays open when every handle to it is gone, still showing what it showed, and opening the key again hands it back instead of opening another. |
| `closes` | `boolean?` | Whether the person asking it to close (its close button, say) closes it; `true` by default. With `false`, the request is only an event, and the application decides: asks to save, say, then calls `close`. |

- **Capability:** `gpu.window`. A script you run holds it, as it holds the GPU; a package target declares it.
- **Errors:** `jess.gpu.window: no window system: …` on a machine without a display (over SSH, say, or in a container), which a later call tries again, as the display may have come back; `windows belong to the thread that opened the first one`, since a process has one window system loop; on macOS, `jess.gpu.window: on macOS, windows open on the process's main thread`, so a worker can't open one there; and a jess built without the `window` feature says so: `jess.gpu.window: this jess has no windows: it was built without the window feature`.

{{ api_signature(value="jess.gpu.pump() -> ()") }}

Handles what the window system reported since the last call (sizes, closes, keys, the pointer), [commits](#commits) what the script changed, and waits for every window to draw it, a quarter of a second at most, so a script's own loop goes at the display's pace. An application's host does the same every frame, around the frame, without the waiting, and its scripts never call it; a script that opens a window itself calls it once a frame.

## GpuWindow

| Member | Type | Holds |
| --- | --- | --- |
| `title` | `string` | Its title. |
| `number` | `number` | Its own number, no other window's in the process: its keyboard and pointer are the input devices whose `backend` is `window:<number>`. |
| `key` | `string?` | The key it is kept by, if it was opened with one. |
| `frames` | `number` | How many frames it has presented: one for each commit, plan shown, and resize there was something new to draw for. |
| `width`, `height` | `number` | Its inner size in physical pixels, which its surface and the plan it shows are sized to. |
| `scale` | `number` | How many physical pixels a logical one is on its display. |
| `format` | `GpuTextureFormat?` | What its images are; `nil` once it's closed. |
| `open` | `boolean` | Whether it's open: neither closed by the person nor by `close`. |
| `show(plan, target, imports?, sets?)` | `(GpuPlan, string, { [string]: GpuTexture \| GpuBuffer }?, GpuDrawSets?) -> ()` | Draws `plan` into it from the next [commit](#commits) on, its image as the plan's imported target `target`, which must have the window's format, with `imports` for the plan's other imported targets and buffers, drawing from `sets` or the device's own. The plan goes with the window: resized with it, so `plan` itself runs, reads back, and reports its size at the window's size from then on, and what it keeps is what the window drew. A plan is shown in one window at a time; showing it in another is refused (`the plan is shown in another window, whose size it takes`), and `plan:resized` makes one of its own. |
| `hide()`, `close()` | `() -> ()` | Stops drawing from the next commit on, or closes it now. |
| `resize(width, height)` | `(number, number) -> ()` | Asks the window system for that inner size in physical pixels. It may give another, or none (a tiling window manager, say); what it gives arrives as a `resized` event, and the shown plan goes with it. |
| `events()` | `() -> { GpuWindowEvent }` | What happened to it since the last call, oldest first, the last 256 at most (a run of pointer moves is the last of them): `{ kind = 'resized', width, height }`, `{ kind = 'scaled', scale }`, `{ kind = 'focused', focused }`, `{ kind = 'closeRequested' }`, and `{ kind = 'closed' }`, and what the person did in it, below. A shown plan is resized already when its `resized` is read; this is for what the application does about it (a camera's aspect, say). |
| `textInput(enabled, area?)` | `(boolean, GpuTextArea?) -> ()` | Asks for text, or stops. While it asks, an input method (the one that writes Japanese, say) may compose for it, showing its candidates beside `area`, `{ x, y, width, height }` in physical pixels from the top left: where the text goes. Keys type `text` whether it asks or not. |

Under an application's host, a window something happened to is announced: the host posts the [named event](@/docs/scripting/applications.md#named-events) [`jess.window`](@/docs/api/events.md#jess-window), `{ title, key }`, once until its events are read, and a handler of it (`on.event['jess.window']`) reads them with `events()`. Nothing runs while nothing happens to a window, and the handler runs in the frame the news came in. A script that drives its own frames reads `events()` when it likes.

A window kept by a key outlives the code that opened it. When an application restarts with new code (its `restart`, a reload), its kept windows stay open through the load, each showing the last frame the old code drew, and the new code's `jess.gpu.window(device, { key = 'main' })` gets its window back, size, place, and all, with the title it asks for. Once the new code has loaded, the kept windows it didn't open again close; if it failed to load, none do, so the next load can take them back. A window the new code reclaims on another device has its surface made again on that one, and shows nothing until the new code gives it a plan. If that device can't present to it, opening it fails and the window stays as it was, on its old device, showing what it showed.

`tostring(window)` is `GpuWindow(title, 800×600)`. Windows draw on a thread of their own, the presenter, never in Luau: when a commit changed something on a window's device, or it was shown a plan or resized, the presenter takes its next image, runs the plan into it with the draw sets as the commit left them, and presents it. A scene that doesn't change needs no new draw. The presenter sees a commit's changes together. An application host does not wait for presentation on each frame; a standalone script's `pump` can wait as described above. A draw that fails presents nothing and isn't counted in `frames`: the presenter tries the window again until it draws, giving it a new surface if its own was lost (the display changed, the driver restarted), and the failure is reported once, by the next `jess.gpu.pump` (`jess.gpu.pump: window Viewer: …`) or, under an application's host, in its log.

What the person does in a window reaches a script two ways, for two kinds of code.

Among the window's `events()`, in the order it happened, which is what a text field, an editor, or anything else about one window reads:

| Event | Fields | Says |
| --- | --- | --- |
| `key` | `key`, `pressed`, `repeated` | A key pressed, held (`repeated`, the system's repeat while it is down), or released while the window has the keyboard, named by position (`KeyW`, `Backspace`, `ArrowLeft`), as bindings name them. |
| `text` | `text` | Text typed: by a key, with the layout and dead keys applied (`é`), or by an input method finishing a composition. Backspace, return, tab, and escape are keys, not text. |
| `preedit` | `text`, `from?`, `to?` | An input method's composition so far, to show where the text goes until it becomes `text`; empty when it is abandoned or done. `text:sub(from, to)` is what its cursor selects, empty for a caret. |
| `inputMethod` | `enabled` | An input method started or stopped composing for the window, which happens only while it asks for text (`textInput`). |
| `pointerEntered`, `pointerLeft` | | The pointer came over the window, or left it. |
| `pointerMoved` | `x`, `y` | Where the pointer is now, in physical pixels from the window's top left. |
| `pointerButton` | `button`, `pressed` | `Primary`, `Secondary`, `Middle`, `Back`, or `Forward`, pressed or released over the window. |
| `wheel` | `x`, `y` | The wheel turned over the window, in lines, `y` positive away from the person. |
| `modifiers` | `shift`, `control`, `alt`, `meta` | Which modifiers are held, on either side, whenever that changes. |

Through [`@jess/input`](@/docs/api/input.md#windows), as devices, which is what bindings read: under the host of an application target that declares `gpu.window`, each window has a keyboard and a pointer of its own, which connect at its first input, disconnect when it closes, and say `backend = 'window:<number>'` in `runtime:devices()`. Keys, pointer buttons, position, motion, and the wheel are named as every jess backend names them (`key:KeyW`, `pointerButton:Primary`, `pointerAxis:DeltaX`). A binding to a control on any device, which is what a game binds, sees every window; one to a device sees one window. A window's pointer is at its place in that window, and at 0, 0 once the pointer leaves, so a binding to any pointer's position follows the window the pointer is over. The mouse's own motion goes to the pointer of the window that has the keyboard. The application is focused while any of its windows has the keyboard. A script you run takes the same from `jess.input.runtime { windows = true }`, its own headless runtime, which polls its windows at every `tick`.

```luau
--!strict
local shader = require '@jess/shader'

local device = jess.gpu.device()
local window = jess.gpu.window(device, { title = 'green', width = 320, height = 240 })

local green = shader.program {
  vertex = function(input)
    local index = input.builtin.vertexIndex
    local corner = shader.vec2(
      shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2))),
      shader.float(shader.bitAnd(index, shader.uint(2)))
    )

    return { position = shader.vec4(corner * 2 - 1, 0, 1) }
  end,
  fragment = function() return shader.vec4(0, 1, 0, 1) end,
}

local format = assert(window.format)
local plan = device:plan {
  viewport = { width = window.width, height = window.height },
  targets = { { name = 'screen', format = format, imported = true } },
  passes = {
    {
      name = 'fill',
      kind = 'fullscreen',
      pipeline = device:pipeline(green, { colors = { format } }),
      colors = { 'screen' },
    },
  },
}

window:show(plan, 'screen')
for _ = 1, 120 do
  jess.gpu.pump()
end
print(window, window.open)
window:close()
-- GpuWindow(green, 320×240)  true
```
