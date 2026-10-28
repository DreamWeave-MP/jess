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
- **Not a wgpu wrapper.** No public type names a wgpu type. `TextureFormat` is jess's own list of the formats jess draws with; a backend maps it to whatever its API calls them. [`jess::gpu`](@/docs/contributing/gpu.md) keeps the same rule for its bind groups, pipelines, and shader modules.
- **Not a Luau API.** Luau describes a graph as a table to [`GpuDevice:plan`](@/docs/api/gpu.md#plans), which builds one of these from it.
- **Not a material or shader system.** A pass says what it touches, not how it shades. Shaders are the [shader IR](@/docs/contributing/shader-ir.md)'s problem.

## Targets and Buffers

A target is a render target (color) or a depth target, with a name, a `TextureFormat`, and a `TargetSize`: absolute pixels, or the viewport times a scale. `TargetSize::resolve` turns a relative size into pixels, rounding to the nearest pixel and never going below one, so a minimized window doesn't ask the backend for a zero-sized texture.

A target also has mip levels, array layers, and samples a pixel, one of each unless its `TargetDesc` says otherwise (`add_target` and `import_target` take one; the `add_render_target` family makes the plain kind). Each level is half the one before, never below a pixel. A multisampled target has 4 samples, one level, and one layer: passes draw into it and resolve it into a single-sample target, and nothing samples or copies it.

Registration rejects an empty or duplicate name, a depth format on a render target (or the reverse), a size of zero pixels or a scale that isn't finite and positive, and a shape that doesn't fit together: no levels or layers, more levels than an absolute size halves to, a sample count other than 1 or 4, or a multisampled target with more than one level or layer.

A relative target with several levels needs pixels for all of them, so `TargetDesc::resolve` never makes its level 0 smaller than 2^(levels - 1) pixels each way: a minimized window still has every level of its bloom chain. Two relative targets are the same size at every viewport only when they have the same scale and the same number of levels, which is what attachments of one pass have to agree on.

A buffer is a name and a size in bytes, a nonzero multiple of four: `add_buffer` for one the graph owns, `import_buffer` for one the host does. Buffers are what compute passes write and what copies read back into.

Targets and buffers are both resources, and each has an origin:

- **Transient** resources belong to the graph. They hold nothing before their first write and nothing after their last use, and they may share memory with other transient resources.
- **Imported** resources belong to the host, like the window's backbuffer, or a buffer the next frame draws instances from. Their contents exist before the graph runs and are observed after it. They're what the graph produces, and they never share memory.
- **Kept** resources belong to whatever runs the graph, a plan. They hold what the last run left, so a first write can load them (a frame's history), and are observed after each run, the way the host reads one back from the plan. They're what the graph produces too, and they never share memory either.

`RenderGraph::add` registers a resource of any description and origin; the `add_*` and `import_*` methods are the common cases. A buffer's `BufferSize` is a number of bytes or, sized by the viewport, room for all of a target of a format at the viewport times a scale, each row padded to `COPY_ROW_ALIGNMENT` as a copy lays it out.

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

A write goes to one level of one layer of a target, level 0 of layer 0 unless `PassOutput::at` names another, and a multisampled color write can resolve (`PassOutput::resolving`) into a level of a layer of a single-sample target of its format and size when the pass ends. A sampled read takes a whole target unless `Pass::read_at` names one level of one layer; a depth test and a copy read one, level 0 of layer 0 unless they name another.

A copy pass reads exactly one source and writes exactly one destination, which can't be cleared, because the copy writes it. It copies one level of one layer, and its `CopyRegion` says what part: where in the source it starts (a texel, or a byte of a buffer), where in the destination it lands, and how much (a rectangle of texels, or bytes between buffers); whatever the region leaves out is the start, and the rest of the source from there. Targets copy into targets of the same format, a depth target only a whole level of a layer at a time. Copies between a target and a buffer need a format whose texels have a layout in a buffer, which `Depth24Plus`'s doesn't, and lay each row out padded to `COPY_ROW_ALIGNMENT`, 256 bytes; nothing copies a buffer into a depth target. `RenderGraph::resolve_copy` resolves a copy to offsets and extents for a viewport; registration checks everything a viewport doesn't change, and every bound of a copy between fixed sizes, so a backend resolves the rest once it has a viewport.

`add_pass` rejects a pass that is wrong on its own: empty name or owner, a duplicate name, no writes, a resource id from another graph, a level or layer its target doesn't have, a read or write the resource's kind or the pass's kind doesn't allow, a multisampled target sampled or copied, a resolve that doesn't fit, a clear value that doesn't fit, the same level of a layer used twice in one direction, a level of a layer both read and written (a feedback loop, or a depth test on the depth it's also writing), more than one depth attachment, attachments of different sizes or sample counts, a copy that doesn't fit, or a copy region on any other pass. Sampled reads aren't attachments, so their size is free.

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

The rules hold for each level of each layer on its own: a pass that reads level 0 of a target and draws into level 1 depends only on the writers of level 0, so a mip chain is a run of passes over one target, each reading the level the last one drew. A read of the whole target follows the last writer of every level of every layer, and needs each written; the error names the one that isn't (`pass "composite" reads level 2 of layer 0 of "chain", which no pass writes`). A dependency names its resource, and two passes are linked once a resource and kind, however many levels link them.

## Culling

Imported and kept resources are what the graph produces, so a pass survives when the host can see its work: it is the last pass to store into an imported or kept target or buffer, or a surviving pass reads or loads what it stored. Everything else is culled.

Culling is not silent. `CompiledGraph::culled` lists the culled passes and the dump prints them. A pass that clears a target that the next writer clears again is dead work, and the dump says so. A graph where nothing reaches an imported target is an error, not an empty frame.

## Lifetimes and Aliasing

Each resource used by a surviving pass gets a `Lifetime`: the positions of its first and last pass in the compiled order, both inclusive. Two lifetimes that share a pass overlap, because both resources are bound during it.

Transient targets with the same description (format, size, levels, layers, and samples), or transient buffers with the same size, and lifetimes that don't overlap, can share one allocation. The compiler walks the resources in order of first use and puts each into the earliest group whose last member is finished before it starts. Every used transient resource lands in exactly one `AliasGroup`; each group is one allocation. In a bloom chain at half resolution, `bright` is done before `blurVertical` starts, so they share:

```text
aliasing
    depth
    scene
    bright, blurVertical
    blurHorizontal
    combined
```

## The Dump

`CompiledGraph`'s `Display` prints the graph's name, every surviving pass in order with where it was declared (`from:`, when its metadata knows) and what it reads and writes, the culled passes if there are any, then every resource with its format (`buffer` for a buffer), size, origin, and the passes that bound its lifetime, then the alias groups. A use of one level or layer of a target that has several says which (`chain[level 1]`, `faces[layer 2]`), a resolve follows its write (`samples -> screen`), and a size goes on with the levels, layers, and samples a target has more than one of (`64x64, 4 levels`):

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
