---
title: 'Render Graph'
description: 'What jess::render is, what it deliberately is not, and how passes, targets, ordering, culling, lifetimes, and aliasing work.'
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

- `RenderGraph` is what gets declared: targets and the passes that use them.
- `CompiledGraph` is what a backend executes: pass order, culled passes, dependencies, target lifetimes, and alias groups. Its `Display` is the debug dump.

The two ends of that line don't exist yet. The technique will eventually come from Luau (`@jess/render`), and the backend will be wgpu. Neither is in this module.

## What It Is Not

- **Not a renderer.** It allocates no textures and records no commands.
- **Not a wgpu wrapper.** No public type names a wgpu type. `TargetFormat` is jess's own list of the formats the renderer needs; a backend maps it to whatever its API calls them. The same goes, later, for bind groups, pipeline layouts, and shader modules.
- **Not a Luau API.** There are no bindings. Those wait for l3i, and a temporary binding layer would become the permanent one the day it shipped.
- **Not a material or shader system.** A pass says what it touches, not how it shades. Shaders are `jess-shader`'s problem.
- **Not published.** The crate is `publish = false` until the native foundations are reassessed.

Compute passes aren't modeled. `PassKind` is non-exhaustive so they can land when a pass needs one.

## Targets

A target is a render target (color) or a depth target, with a name, a `TargetFormat`, and a `TargetSize`: absolute pixels, or the viewport times a scale. `TargetSize::resolve` turns a relative size into pixels, rounding to the nearest pixel and never going below one, so a minimized window doesn't ask the backend for a zero-sized texture.

Registration rejects an empty or duplicate name, a depth format on a render target (or the reverse), and a size of zero pixels or a scale that isn't finite and positive.

Targets have an origin:

- **Transient** targets belong to the graph. They hold nothing before their first write and nothing after their last use, and they may share memory with other transient targets.
- **Imported** targets belong to the host, like the window's backbuffer. Their contents exist before the graph runs and are observed after it. They're what the graph produces, and they never share memory.

## Passes

A pass has a kind (`Mesh` or `Fullscreen`), metadata, reads, and writes.

The metadata is for profilers and graph inspectors: a name unique in the graph, an owner (the package or subsystem that declared it), and an optional source location. The IR knows nothing about Luau; a Luau front end fills in `file:line:column` and the IR carries it.

Reads come in three kinds, and the target's kind has to match:

| Read | Allowed on |
| --- | --- |
| `Texture` | render targets |
| `DepthTexture` | depth targets |
| `DepthTest` (the read-only depth attachment) | depth targets |

Writes go through a color attachment (render targets) or the depth attachment (depth targets), with a `LoadOp` (`Clear(value)`, `Load`, `DontCare`) and a `StoreOp` (`Store`, `Discard`). A clear value has to fit the format: `Color` for float and normalized formats, `Uint` for `R32Uint` picking ids, `Depth` within `0.0..=1.0` for depth.

`add_pass` rejects a pass that is wrong on its own: empty name or owner, a duplicate name, no writes, a target id from another graph, a read or write the target's kind doesn't allow, a clear value that doesn't fit, the same target used twice, a target both read and written (a feedback loop, or a depth test on the depth it's also writing), more than one depth attachment, or attachments of different sizes. Sampled reads aren't attachments, so their size is free.

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

Compiling also rejects a transient target no pass uses and a read of a transient target no pass writes.

## Culling

Imported targets are what the graph produces, so a pass survives when the host can see its work: it is the last pass to store into an imported target, or a surviving pass reads or loads what it stored. Everything else is culled.

Culling is not silent. `CompiledGraph::culled` lists the culled passes and the dump prints them. A pass that clears a target that the next writer clears again is dead work, and the dump says so. A graph where nothing reaches an imported target is an error, not an empty frame.

## Lifetimes and Aliasing

Each target used by a surviving pass gets a `Lifetime`: the positions of its first and last pass in the compiled order, both inclusive. Two lifetimes that share a pass overlap, because both targets are bound during it.

Transient targets with the same kind, format, and size, and lifetimes that don't overlap, can share one allocation. The compiler walks the targets in order of first use and puts each into the earliest group whose last member is finished before it starts. Every used transient target lands in exactly one `AliasGroup`; each group is one allocation. In a bloom chain at half resolution, `bright` is done before `blurVertical` starts, so they share:

```text
aliasing
    depth
    scene
    bright, blurVertical
    blurHorizontal
    combined
```

## The Dump

`CompiledGraph`'s `Display` prints the graph's name, every surviving pass in order with what it reads and writes, the culled passes if there are any, then every target with its format, size, origin, and the passes that bound its lifetime, then the alias groups:

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
cargo test --test render
```

The representative golden dumps live in `tests/render/golden`. `JESS_BLESS=1` rewrites them; only bless when every change to the dump is intended, and say why in the commit.
