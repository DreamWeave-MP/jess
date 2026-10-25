---
title: 'jess.gpu'
description: 'The GPU in front of the script: the device jess opens on it, what it reports, and the buffers, textures, and samplers made on it.'
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
