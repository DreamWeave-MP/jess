---
title: 'Shader IR'
description: 'What jess-shader is, what it deliberately is not, and how its types, validation, source attribution, constant folding, and WGSL lowering work.'
weight: 70
extra:
  kind: guide
---

A lit, textured NIF shape needs about one line of shading: sample the diffuse texture, scale it by how much the surface faces the light. Written as WGSL, that line drags along a uniform struct with the right padding, a bind group number, a binding number for the texture, another for the sampler, attribute locations, and an interface struct the vertex and fragment stages have to agree on. None of that is shading. All of it is somewhere to be wrong.

`jess-shader` (`libs/shader`) keeps the one line and takes the rest. A program says what it computes. jess picks every binding and location, checks the program in its own terms, and writes the WGSL.

## What It Is

A plain Rust IR with no runtime dependencies:

```text
shader expressions -> Program -> validate -> fold_constants -> wgsl::lower -> WgslModule -> backend
```

- `Program` is an arena of `Expression`s plus a vertex stage (the clip position and any varyings) and a fragment stage (color outputs).
- `Program::validate` finds every mistake it can before any backend sees the program.
- `Program::fold_constants` folds the trivial stuff, and nothing more.
- `wgsl::lower` writes one WGSL module and reports every binding, uniform layout, and attribute location the renderer has to match.

The Luau end (`@jess/shader`) doesn't exist yet, and neither does the renderer that consumes the module. Neither is in this crate.

## What It Is Not

- **Not a WGSL API.** Nothing here takes WGSL. WGSL is backend output: dump it when debugging, never write it by hand. A raw escape hatch may exist someday if something proves it has to.
- **Not a wgpu wrapper.** No public type names a wgpu or naga type. naga and wgpu are dev-dependencies, used only to prove the output is valid and draws.
- **Not a Luau API.** There are no bindings. Those wait for l3i.
- **Not a render graph or a material system.** Which passes run and what feeds them is `jess-render`'s problem. What a NIF property means is `NifScene`'s.
- **Not an optimizing compiler.** naga and the driver already are one.
- **Not published.** The crate is `publish = false` until the native foundations are reassessed.

## Types

Exactly what the NIF renderer needs:

| Type | Notes |
| --- | --- |
| `Bool`, `Int`, `UInt`, `Float` | 32-bit scalars |
| `Vec2f`, `Vec3f`, `Vec4f` | float vectors only; nothing needs integer vectors yet |
| `Mat3f`, `Mat4f` | column-major float matrices |
| `Texture2D`, `Sampler` | resources, never values: only a parameter has these types |

Not the entire WGSL type system. If a first-party shader needs another type, it gets added then.

## Expressions

Expressions live in an arena and name each other by `ExpressionId`. Ids only point backwards: an operand always comes before the expression that reads it. The arena is already in dependency order, so validation, folding, and lowering are single passes over a vector. No recursion, no boxed trees, no visitor.

The leaves are literals, declared parameters, vertex inputs, varyings, and semantics. The operations are `sample`, `+ - * /`, `min`, `max`, `clamp`, `dot`, `cross`, `normalize`, `length`, `mix`, vector constructors, and swizzles. `*` covers matrix × vector and matrix × matrix. `min`, `max`, `clamp`, and `mix` take a `Float` where a vector is expected, because every shader author writes `clamp(color, 0.0, 1.0)` and WGSL refusing it helps nobody.

Semantics are values the renderer provides, named by what they mean rather than where they live:

| Semantic | Type | Lowered |
| --- | --- | --- |
| `model`, `view`, `projection` | `Mat4f` | yes |
| `cameraPosition` | `Vec3f` | yes |
| `viewportSize` | `Vec2f` | yes |
| `time` | `Float` | yes |
| `sceneColor`, `sceneDepth`, `previousPass` | `Texture2D` | not yet |

The pass inputs exist in the IR so the render graph can grow into them. Until the lowering can bind them, validation says so instead of generating something that half works.

## Validation

`Program::validate` returns every problem it finds, in Jess terms, as a typed `ValidationErrorKind`:

```text
dot(Vec3f, Vec4f) is invalid: expected two float vectors of the same size
sample(Texture2D, Sampler, Vec3f) is invalid: expected sample(Texture2D, Sampler, Vec2f)
fragment output 0 is Vec3f: expected Vec4f
varying "uv" is read by the fragment stage but never written by the vertex stage
vertex input "position" is read in the fragment stage: pass it through a varying
semantic sceneColor is not supported by the WGSL lowering yet
```

It checks declarations (names, duplicates, and types their kind can't have, such as a `Bool` parameter, which no uniform buffer can hold), references to anything undeclared or from another program, operand types, which stage reads what (vertex inputs are vertex-only, varyings fragment-only, and `sample` needs the fragment stage's derivatives), the stage interface, and output types. Constant arithmetic that WGSL would reject while compiling (integer overflow, integer division by zero, a non-finite float) is rejected here first, along with NaN and infinite literals, which WGSL can't write at all.

Errors are collected, not thrown one at a time. An expression whose operand already failed is left untyped and not reported again, so one bad `dot` is one error, not one per expression downstream of it. Expressions no stage reads aren't checked. They aren't lowered either.

naga validating the output is a test, not a diagnostic. If naga ever rejects a program validation accepted, that's a jess bug.

## Source Attribution

Every expression and declaration can carry a `SourceOrigin`: owner, file, line, and column. `push_at`, `parameter_at`, `vertex_input_at`, and `varying_at` record one; plain `push` doesn't. Origins are stored sparsely beside the arena, so native Rust that never records one pays nothing for them.

Errors carry the origin of the expression or declaration at fault:

```text
shaders/lit.luau:12:5 (@jess/nif-viewer): dot(Vec3f, Vec4f) is invalid: expected two float vectors of the same size
```

The Luau front end will fill these in. The IR doesn't know what Luau is; it just refuses to be structurally incapable of pointing at it.

## Constant Folding

`fold_constants` folds constant arithmetic (per component, with a `Float` broadcast against a vector), vector constructors of constants, and the identities `x * 1`, `1 * x`, `x / 1`, `x + 0`, `0 + x`, and `x - 0`, where the result has `x`'s type. `M * vec4(1.0)` is not `M`.

A folded expression becomes a literal in place. An identity's readers read `x` instead. Ids stay valid. The program is validated first and left untouched when it's invalid. Folding is a separate step: `wgsl::lower` lowers exactly what it's given.

## Lowering

One module writes all WGSL, with one naming scheme:

| What | WGSL name |
| --- | --- |
| value parameter `color` | `parameters.p_color` |
| texture and sampler `diffuse` | `t_diffuse`, `s_diffuse` |
| vertex input `uv` | `input.a_uv` |
| varying `uv` | `input.v_uv` |
| semantic `cameraPosition` | `semantics.camera_position` |
| fragment output 0 | `color0` |
| temporaries | `e0`, `e1`, ... per entry point, in arena order |

Every non-leaf expression a stage reads becomes one `let`. The entry points are `vertex_main` and `fragment_main`. The same program always lowers to the same bytes.

Bindings belong to jess, never the author. Semantics are one uniform block at `@group(0) @binding(0)`, holding only the semantics the program reads. Value parameters are one uniform block at `@group(1) @binding(0)`, in declaration order, and textures and samplers follow in `@group(1)`. Vertex inputs get locations in declaration order. `WgslModule` reports all of it, including each uniform member's offset and size under WGSL's layout rules (a `Vec3f` aligns to 16 bytes, a `Mat3f` is three padded 16-byte columns), so the renderer never has to rediscover the layout from the source.

The fragment half of the textured, lit golden:

```wgsl
@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    let e0 = textureSample(t_diffuse, s_diffuse, input.v_uv);
    let e1 = normalize(input.v_normal);
    let e2 = dot(e1, parameters.p_lightDirection);
    let e3 = max(e2, 0.0);
    let e4 = e3 + parameters.p_ambient;
    let e5 = e0.xyz;
    let e6 = e5 * e4;
    let e7 = clamp(e6, vec3<f32>(0.0), vec3<f32>(1.0));
    let e8 = e0.w;
    let e9 = vec4<f32>(e7, e8);
    var output: FragmentOutput;
    output.color0 = e9;
    return output;
}
```

## Testing

```sh
cargo test -p jess-shader -- --nocapture
```

Every lowered module in the tests goes through naga's parser and validator, and the uniform layouts jess reports are checked against the ones naga computes. The golden WGSL lives in `libs/shader/tests/golden`; `JESS_BLESS=1` rewrites it. Only bless when every change to the output is intended, and say why in the commit.

`tests/triangle.rs` is the proof the layers compose: it builds a triangle program in the IR, lowers it, renders it headless with wgpu into an offscreen texture using only the bindings and locations `WgslModule` reported, and reads the center and corner pixels back. On a machine with no adapter at all, not even a software one, it prints `SKIPPED` and passes; `--nocapture` shows which adapter ran it.
