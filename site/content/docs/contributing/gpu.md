---
title: 'GPU'
description: 'What jess::gpu is, why it is wgpu behind a feature, and how devices, buffers, textures, and their errors work.'
weight: 65
extra:
  kind: guide
---

The [render graph](@/docs/contributing/render-graph.md) says what to draw in what order, and the [shader IR](@/docs/contributing/shader-ir.md) says how each draw shades. Something has to own the hardware they run on. `jess::gpu` (`src/gpu`) is that: one adapter, the device opened on it, and the bytes and images on that device.

## What It Is

- **A device.** `Gpu::headless()` takes whatever adapter the machine has, preferring a hardware one and falling back to a software one, and opens a device on it. `Gpu::info()` reports the adapter's name, the API it's reached through (`vulkan`, `metal`, `dx12`, `gl`), what kind of device it is, and the limits a caller sizes things by.
- **Buffers.** Bytes on the GPU, bound as any of vertex, index, uniform, and storage buffers, written from the CPU and read back. Sizes, offsets, and lengths are whole 4-byte words, which is what the GPU copies.
- **Textures.** 2D, layered, cube, and 3D images in a [`TextureFormat`](@/docs/contributing/render-graph.md#targets), with mip levels, sampled by shaders, drawn into as targets, or both. A level of a layer is written from tightly packed rows and read back the same way; jess pads and unpads the rows the GPU's copies need.

## What It Is Not

- **Not a wgpu wrapper.** No public type names a wgpu type. Usages are jess's own small sets, formats are jess's list, and errors are jess's.
- **Not a renderer.** It doesn't know what a mesh, a camera, or a material is. What a buffer holds and why is the caller's business, and in the end Luau's.
- **Not on by default.** It's behind the `gpu` feature, so the kernel still builds and runs without a GPU stack.

## Why wgpu

wgpu is the one GPU API that reaches Vulkan, Metal, Direct3D 12, and OpenGL from one Rust interface, validates what it's given instead of crashing the driver, and speaks WGSL, which is what the shader IR lowers to. jess takes wgpu 30 with only the native backends (`vulkan`, `gles`, `metal`, `dx12`) and `wgsl`, and pollster to wait on its futures. Both, and everything they pull in, are MIT, Apache-2.0, Zlib, ISC, or BSD-2-Clause; the native pieces (the Vulkan loader, GL and EGL, Metal, Direct3D) are the system's own, loaded when the device opens.

## Errors

A call that can fail checks its arguments in jess's terms first: a buffer of 6 bytes, a cube with oblong faces, a level a texture doesn't have, bytes that don't fill the level they're for. Each is a `GpuError::Invalid` naming what's wrong and what was expected. What gets past that runs inside a wgpu error scope, so anything wgpu refuses comes back as `GpuError::Backend` with wgpu's message, rather than through wgpu's default handler, which panics.

## Testing

The tests are in `tests/gpu`, behind the feature: `cargo test --test integration --features gpu -- gpu::`. Each runs on whatever adapter the machine has, a software one (lavapipe, llvmpipe, WARP) included, and skips with a `SKIPPED` line on a machine without one.
