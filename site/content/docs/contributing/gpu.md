---
title: 'GPU'
description: 'What jess::gpu is, why it is wgpu behind a feature, and how devices, resources, pipelines, bind groups, passes, and their errors work.'
weight: 65
extra:
  kind: guide
---

The [render graph](@/docs/contributing/render-graph.md) says what to draw in what order, and the [shader IR](@/docs/contributing/shader-ir.md) says how each draw shades. Something has to own the hardware they run on. `jess::gpu` (`src/gpu`) is that: one adapter, the device opened on it, the bytes and images on that device, and the pipelines and passes that draw with them.

## What It Is

- **A device.** `Gpu::headless()` takes whatever adapter the machine has, preferring a hardware one and falling back to a software one, and opens a device on it. `Gpu::info()` reports the adapter's name, the API it's reached through (`vulkan`, `metal`, `dx12`, `gl`), what kind of device it is, and the limits a caller sizes things by.
- **Buffers.** Bytes on the GPU, bound as any of vertex, index, uniform, and storage buffers, written from the CPU and read back. Sizes, offsets, and lengths are whole 4-byte words, which is what the GPU copies.
- **Textures.** 2D, layered, cube, and 3D images in a [`TextureFormat`](@/docs/contributing/render-graph.md#targets), with mip levels, sampled by shaders, drawn into as targets, or both. A level of a layer is written from tightly packed rows and read back the same way; jess pads and unpads the rows the GPU's copies need.
- **Samplers.** Nearest or linear filtering, between texels and between mip levels, clamping, repeating, or mirroring past the edges, and an optional comparison for depth textures.
- **Pipelines.** `Gpu::render_pipeline(program, state)` takes a [shader program](@/docs/contributing/shader-ir.md) and its fixed-function state (topology, culling, which way round faces the camera, a color target per fragment output, and an optional depth test) and builds everything else from the program's own layout: a bind group layout per frequency, the vertex buffer layouts per rate, a matrix attribute as a column per location. The state's color targets must fit the outputs: a `UInt` output needs an unsigned integer target, and a target can't have more channels than its output.
- **Bind groups.** `Gpu::bind_group(pipeline, frequency, bindings)` binds exactly what the program declares for that frequency, each binding checked against its declaration: a uniform buffer at least as large as the block, a texture of the declared shape and of a format the declaration reads, a comparison sampler where one is declared.
- **Compute.** `Gpu::compute_pipeline(program)` takes a compute program the same way, `Gpu::compute_bind_group` binds for it, and `Gpu::dispatch(pipeline, groups, workgroups)` runs it over a grid of workgroups, each of the size the program declares.
- **Passes.** `Gpu::render(colors, depth, draws)` draws into a level of a layer of each target, starting from a clear or what the target held, with the render graph's own `LoadOp`, `StoreOp`, and `ClearValue`. Each draw is checked before anything is recorded: its pipeline's targets are the pass's, it has a bind group for every frequency its program binds and none for the rest, its buffers are vertex and index buffers, and its count stays inside them.

## What It Is Not

- **Not a wgpu wrapper.** No public type names a wgpu type. Usages are jess's own small sets, formats are jess's list, and errors are jess's.
- **Not a renderer.** It doesn't know what a mesh, a camera, or a material is. What a buffer holds and why is the caller's business, and in the end Luau's.
- **Not on by default.** It's behind the `gpu` feature, so the kernel still builds and runs without a GPU stack.

## Why wgpu

wgpu is the one GPU API that reaches Vulkan, Metal, Direct3D 12, and OpenGL from one Rust interface, validates what it's given instead of crashing the driver, and speaks WGSL, which is what the shader IR lowers to. jess takes wgpu 30 with only the native backends (`vulkan`, `gles`, `metal`, `dx12`) and `wgsl`, and pollster to wait on its futures. Both, and everything they pull in, are MIT, Apache-2.0, Zlib, ISC, or BSD-2-Clause; the native pieces (the Vulkan loader, GL and EGL, Metal, Direct3D) are the system's own, loaded when the device opens.

## Errors

A call that can fail checks its arguments in jess's terms first: a buffer of 6 bytes, a cube with oblong faces, a level a texture doesn't have, bytes that don't fill the level they're for, a `Vec4f` output drawn into an `R32Uint` target, a draw without the material bind group its program needs. Each is a `GpuError::Invalid` naming what's wrong and what was expected. What gets past that runs inside a wgpu error scope, so anything wgpu refuses comes back as `GpuError::Backend` with wgpu's message, rather than through wgpu's default handler, which panics.

## Testing

The tests are in `tests/gpu`, behind the feature: `cargo test --test integration --features gpu -- gpu::`. Each runs on whatever adapter the machine has, a software one (lavapipe, llvmpipe, WARP) included, and skips with a `SKIPPED` line on a machine without one.
