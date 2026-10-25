---
title: 'Render Graph'
description: 'What jess::render is, what it deliberately is not, and how passes, targets, buffers, ordering, culling, lifetimes, and aliasing work.'
weight: 60
extra:
  kind: guide
---

A mesh viewer draws depth, then opaque geometry, then a selection mask, then an outline over the result. Four passes, four targets. Hardcode that sequence and the first package that wants a normals view, a heatmap, or a wireframe between two of those passes has to know every pass that already exists and where it sits.

`jess::render` (`src/render`) exists so nobody has to know that. Passes say what they read and write. The graph works out the order, checks that it makes sense, and says which targets can share memory.

## What It Is

A plain Rust intermediate representation, with no dependencies:

```text
render technique -> RenderGraph -> CompiledGraph -> backend
```

- `RenderGraph` is what gets declared: targets, buffers, and the passes that use them.
- `CompiledGraph` is what a backend executes: pass order, culled passes, dependencies, resource lifetimes, and alias groups. Its `Display` is the debug dump.

The backend is [`jess::gpu`](@/docs/contributing/gpu.md#plans): `Gpu::plan` compiles a graph for a device and allocates what it needs, and `Gpu::run` records it. Neither is in this module.

## What It Is Not

- **Not a renderer.** It allocates no textures and records no commands.
- **Not a wgpu wrapper.** No public type names a wgpu type. `TextureFormat` is jess's own list of the formats the renderer needs; a backend maps it to whatever its API calls them. The same goes, later, for bind groups, pipeline layouts, and shader modules.
- **Not a Luau API.** Luau describes a graph as a table to [`GpuDevice:plan`](@/docs/api/gpu.md#plans), which builds one of these from it.
- **Not a material or shader system.** A pass says what it touches, not how it shades. Shaders are the [shader IR](@/docs/contributing/shader-ir.md)'s problem.
- **Not published.** The crate is `publish = false` until the native foundations are reassessed.

## Targets and Buffers

A target is a render target (color) or a depth target, with a name, a `TextureFormat`, and a `TargetSize`: absolute pixels, or the viewport times a scale. `TargetSize::resolve` turns a relative size into pixels, rounding to the nearest pixel and never going below one, so a minimized window doesn't ask the backend for a zero-sized texture.

Registration rejects an empty or duplicate name, a depth format on a render target (or the reverse), and a size of zero pixels or a scale that isn't finite and positive.

A buffer is a name and a size in bytes, a nonzero multiple of four: `add_buffer` for one the graph owns, `import_buffer` for one the host does. Buffers are what compute passes write and what copies read back into.

Targets and buffers are both resources, and each has an origin:

- **Transient** resources belong to the graph. They hold nothing before their first write and nothing after their last use, and they may share memory with other transient resources.
- **Imported** resources belong to the host, like the window's backbuffer, or a buffer the next frame draws instances from. Their contents exist before the graph runs and are observed after it. They're what the graph produces, and they never share memory.

## Passes

A pass has a kind, metadata, reads, and writes. The kind says what the pass does, and what it may read and write:

| Kind | Does | Reads | Writes |
| --- | --- | --- | --- |
| `Mesh` | draws scene geometry | textures, depth tests, buffers | color and depth attachments |
| `Fullscreen` | draws one triangle over its attachments | textures, depth tests, buffers | color and depth attachments |
| `Compute` | runs a compute program | textures, buffers | storage buffers |
| `Copy` | copies one resource into another | its source | its destination |

The metadata is for profilers and graph inspectors: a name unique in the graph, an owner (the package or subsystem that declared it), and an optional source location. The IR knows nothing about Luau; a Luau front end fills in `file:line:column` and the IR carries it.

Reads come in five kinds, and the resource's kind has to match:

| Read | Allowed on |
| --- | --- |
| `Texture` | render targets |
| `DepthTexture` | depth targets |
| `DepthTest` (the read-only depth attachment) | depth targets |
| `Buffer` (a storage buffer its shaders read) | buffers |
| `Copy` (a copy's source) | anything |

Writes go through a color attachment (render targets), the depth attachment (depth targets), a storage binding (`Storage`, buffers), or a copy's destination (`Copy`, anything), with a `LoadOp` (`Clear(value)`, `Load`, `DontCare`) and a `StoreOp` (`Store`, `Discard`). A clear value has to fit: `Color` for float and normalized formats, `Uint` for unsigned integer formats such as `R32Uint` picking ids, `Sint` for signed integer formats, `Depth` within `0.0..=1.0` for depth, and `Zero` for a buffer, the only value one clears to. A compute pass that writes a buffer with `Load` continues the previous writer's contents, the way a draw does a target's.

A copy pass reads exactly one source and writes exactly one destination, which can't be cleared, because the copy writes it. A target copies into a target of the same format and size, and a buffer into a buffer at least as large. Copies between a target and a buffer need a format whose texels have a layout in a buffer, which `Depth24Plus`'s doesn't; how many bytes a row takes is the backend's rule, so the backend checks that the buffer is big enough.

`add_pass` rejects a pass that is wrong on its own: empty name or owner, a duplicate name, no writes, a resource id from another graph, a read or write the resource's kind or the pass's kind doesn't allow, a clear value that doesn't fit, the same resource used twice, a resource both read and written (a feedback loop, or a depth test on the depth it's also writing), more than one depth attachment, attachments of different sizes, or a copy that doesn't fit. Sampled reads aren't attachments, so their size is free.

## Ordering

Passes are ordered by what they touch, not by the order they were registered in. Three rules, and one non-rule:

- **Read after write.** A read sees a target's final contents. Every pass that reads a target runs after the last pass that writes it.
- **Write after write.** Passes that write the same target run in registration order. This is the one place registration order carries meaning. A write with `LoadOp::Load` continues the previous writer's contents (opaque, then transparent, into the same scene color); any other load op replaces them.
- **Load needs contents.** The first write of a transient target can't `Load`: there is nothing to load. Nothing may read or load contents the previous writer discarded.
- **Write after read is never an edge.** Every reader already follows every writer. If a graph needs a target written after something read it, that's a cycle, and compiling fails with the cycle's passes named:

```text
dependency cycle: tonemap -> grade -> tonemap
```

The fix is a second target. That isn't a memory tax: aliasing, below, gives the memory back when the lifetimes allow it.

Where the rules leave passes unordered, the earliest-registered ready pass goes first. The same graph always compiles to the same order. Register `mesh.standard` backwards and `depth` still runs first and `outline` last, because the reads force it; `selection` and `opaque` swap, because nothing orders them and registration breaks the tie.

Buffers follow the same rules as targets. Compiling also rejects a transient resource no pass uses and a read of a transient resource no pass writes.

## Culling

Imported resources are what the graph produces, so a pass survives when the host can see its work: it is the last pass to store into an imported target or buffer, or a surviving pass reads or loads what it stored. Everything else is culled.

Culling is not silent. `CompiledGraph::culled` lists the culled passes and the dump prints them. A pass that clears a target that the next writer clears again is dead work, and the dump says so. A graph where nothing reaches an imported target is an error, not an empty frame.

## Lifetimes and Aliasing

Each resource used by a surviving pass gets a `Lifetime`: the positions of its first and last pass in the compiled order, both inclusive. Two lifetimes that share a pass overlap, because both resources are bound during it.

Transient targets with the same format and size, or transient buffers with the same size, and lifetimes that don't overlap, can share one allocation. The compiler walks the resources in order of first use and puts each into the earliest group whose last member is finished before it starts. Every used transient resource lands in exactly one `AliasGroup`; each group is one allocation. In a bloom chain at half resolution, `bright` is done before `blurVertical` starts, so they share:

```text
aliasing
    depth
    scene
    bright, blurVertical
    blurHorizontal
    combined
```

## The Dump

`CompiledGraph`'s `Display` prints the graph's name, every surviving pass in order with where it was declared (`from:`, when its metadata knows) and what it reads and writes, the culled passes if there are any, then every resource with its format (`buffer` for a buffer), size, origin, and the passes that bound its lifetime, then the alias groups:

```text
mesh.standard

depth
    writes: depth

opaque
    reads:  depth
    writes: sceneColor

selection
    reads:  depth
    writes: selectionMask

outline
    reads:  sceneColor, selectionMask
    writes: output

resources
    depth          Depth32Float    viewport  transient  depth .. selection
    sceneColor     Rgba16Float     viewport  transient  opaque .. outline
    selectionMask  R8Unorm         viewport  transient  selection .. outline
    output         Bgra8UnormSrgb  viewport  imported   outline

aliasing
    depth
    sceneColor
    selectionMask
```

The dump is deterministic, and it's golden-tested. It's the first graph inspector jess has, and it doesn't need a GUI.

## Testing

```sh
cargo test --test integration render::
```

The representative golden dumps live in `tests/render/golden`. `JESS_BLESS=1` rewrites them; only bless when every change to the dump is intended, and say why in the commit.
