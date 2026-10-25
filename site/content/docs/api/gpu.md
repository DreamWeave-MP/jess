---
title: 'jess.gpu'
description: 'The GPU in front of the script: the device jess opens on it, and what it reports.'
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
