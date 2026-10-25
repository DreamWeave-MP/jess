---
title: 'Shader IR'
description: 'What jess::shader is, what it deliberately is not, and how its types, validation, source attribution, constant folding, and WGSL lowering work.'
weight: 70
extra:
  kind: guide
---

A lit, textured surface needs about one line of shading: sample the diffuse texture, scale it by how much the surface faces the light. Written as WGSL, that line drags along a uniform struct with the right padding, a bind group number, a binding number for the texture, another for the sampler, attribute locations, and an interface struct the vertex and fragment stages have to agree on. None of that is shading. All of it is somewhere to be wrong.

`jess::shader` (`src/shader`) keeps the one line and takes the rest. A program says what it computes. jess picks every binding and location, checks the program in its own terms, and writes the WGSL.

## What It Is

A plain Rust IR with no runtime dependencies:

```text
shader expressions -> Program -> validate -> fold_constants -> wgsl::lower -> WgslModule -> backend
```

- `Program` is an arena of `Expression`s plus a vertex stage (the clip position and any varyings) and a fragment stage (color outputs).
- `Program::validate` finds every mistake it can before any backend sees the program.
- `Program::fold_constants` folds the trivial stuff, and nothing more.
- `wgsl::lower` writes one WGSL module and reports every binding, uniform layout, and attribute location the renderer has to match.

The Luau end is [`jess.shader`](@/docs/api/shader.md): an ordinary Luau module runs once, calls the builders, and returns a validated program in this IR. The renderer that consumes the lowered module is not in the kernel; the [render graph](@/docs/contributing/render-graph.md) is where a pass says what it touches.

## What It Is Not

- **Not a WGSL API.** Nothing here takes WGSL. WGSL is backend output: dump it when debugging, never write it by hand. A raw escape hatch may exist someday if something proves it has to.
- **Not a wgpu wrapper.** No public type names a wgpu or naga type. naga and wgpu are dev-dependencies, used only to prove the output is valid and draws.
- **Not a render graph or a material system.** Which passes run and what feeds them is the render graph's problem. What a surface's properties mean is the package that owns the format's.
- **Not an optimizing compiler.** naga and the driver already are one.

## Types

Exactly what a first renderer needs:

| Type | Notes |
| --- | --- |
| `Bool`, `Int`, `UInt`, `Float` | 32-bit scalars |
| `Vec2f`, `Vec3f`, `Vec4f` | float vectors |
| `Vec2i`..`Vec4i`, `Vec2u`..`Vec4u` | signed and unsigned integer vectors; varyings of them pass flat |
| `Mat3f`, `Mat4f` | column-major float matrices |

Not the entire WGSL type system. If a first-party shader needs another type, it gets added then. Structs, arrays, textures, and samplers aren't values at all: a program declares them (see [Layout](#layout)), and expressions read the values in them.

## Expressions

Expressions live in an arena and name each other by `ExpressionId`. Ids only point backwards: an operand always comes before the expression that reads it. The arena is already in dependency order, so validation, folding, and lowering are single passes over a vector. No recursion, no boxed trees, no visitor.

The leaves are literals, attributes, built-in values, varyings, and loads of uniform values. A built-in value is what the pipeline gives a stage: `VertexIndex` and `InstanceIndex` in the vertex stage, `Position` and `FrontFacing` in the fragment stage, and validation refuses one read in the other. The operations are `sample`, the binary operators (`+ - * / %`, the comparisons, logical and and or, the bitwise operators, and the shifts), the unary ones (negation, logical not, bitwise not), `select`, the built-in functions (WGSL's, by its names: `min` and `max`, rounding, roots, powers and logarithms, trigonometry, `step` and `smoothstep`, `mix`, the geometric ones from `dot` to `refract`, `transpose` and `determinant`, and the derivatives), constructors (a vector from components, a matrix from its columns), conversions between scalars that keep a value's size (`Float(index)`, `Vec3f(cell)`), and swizzles. A comparison takes two scalars and gives a `Bool`; there are no vectors of `Bool`, because nothing needs them. `*` covers matrix × vector and matrix × matrix. `min`, `max`, and `clamp` take a scalar of a vector's kind where the vector is expected, `pow`, `atan2`, `step`, and `smoothstep` a `Float` beside a float vector, and `mix` a `Float` amount, because every shader author writes `clamp(color, 0.0, 1.0)` and WGSL refusing it helps nobody. Integer vectors get the same arithmetic, constructors, and swizzles as float vectors; what only makes sense for floats (`normalize`, `length`, `floor`, `mix`, `cross`, and the rest) refuses them. The derivatives, `dpdx`, `dpdy`, and `fwidth`, exist only in the fragment stage, as sampling does.

A load reads a value through a `Place`: a frequency's uniform block, then fields and indices down to one value, `pass.lights[i].color` being the pass block, member `lights`, element `i`, field `color`. An index is an `Int` or `UInt` expression, so a place can depend on what the program computes; a load whose place has an index is computed once, like any other operation, and one without is written where it's read. `sample` names its texture and sampler by the resources the program declared, never by a value.

Nothing else is built in. There's no model matrix, no camera, no time: a program that wants one declares it, as a uniform member some frequency binds, and so the IR holds no notion of what a camera is.

## Validation

`Program::validate` returns every problem it finds, in Jess terms, as a typed `ValidationErrorKind`:

```text
dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type
sample(Texture2D, Sampler, Vec3f) is invalid: expected sample(Texture2D, Sampler, Vec2f)
fragment output 0 is Vec3f: expected Vec4f
varying "uv" is read by the fragment stage but never written by the vertex stage
attribute "position" is read in the fragment stage: pass it through a varying
a load reads element 8 of an array of 4
uniform material.weights is an array of elements 4 bytes apart: a uniform array's elements must be a multiple of 16 bytes apart, so give it 16-byte elements (a Vec4f, a struct), or read it from a storage buffer
```

It checks declarations (names, duplicates, and types their kind can't have, such as a `Bool` uniform member, which no buffer can hold, or a uniform array whose elements aren't 16 bytes apart), references to anything undeclared or from another program, every load's place, operand types, which stage reads what (attributes are vertex-only, varyings fragment-only, and `sample` needs the fragment stage's derivatives), the stage interface, and output types. Constant arithmetic that WGSL would reject while compiling (integer overflow, integer division by zero, a non-finite float) is rejected here first, along with NaN and infinite literals, which WGSL can't write at all.

Errors are collected, not thrown one at a time. An expression whose operand already failed is left untyped and not reported again, so one bad `dot` is one error, not one per expression downstream of it. Expressions no stage reads aren't checked. They aren't lowered either.

naga validating the output is a test, not a diagnostic. If naga ever rejects a program validation accepted, that's a jess bug.

## Source Attribution

Every expression and declaration can carry a `SourceOrigin`: owner, file, line, and column. `push_at` and `varying_at` record one, as does an `origin` on a declaration such as an `Attribute` or a `Uniform`; plain `push` doesn't. Origins are stored sparsely beside the arena, so native Rust that never records one pays nothing for them.

Errors carry the origin of the expression or declaration at fault:

```text
shaders/lit.jsl:12:5 (com.example.viewer): dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type
```

The Luau front end fills these in from the module that built the expression. The IR doesn't know what Luau is; it just refuses to be structurally incapable of pointing at it.

## Constant Folding

`fold_constants` folds constant arithmetic (per component, with a `Float` broadcast against a vector), vector constructors of constants, and the identities `x * 1`, `1 * x`, `x / 1`, `x + 0`, `0 + x`, and `x - 0`, where the result has `x`'s type. `M * vec4(1.0)` is not `M`.

A folded expression becomes a literal in place. An identity's readers read `x` instead. Ids stay valid. The program is validated first and left untouched when it's invalid. Folding is a separate step: `wgsl::lower` lowers exactly what it's given.

## Layout

A program declares structs, vertex attributes, uniform members, and textures and samplers, each by what it is and how often it changes: an attribute advances per vertex or per instance, and a uniform member or resource is bound per pass, per material, or per draw (the three `Frequency` values). Where any of it lives is `jess::shader::layout`'s decision, and `Program::layout` asks for it. Nothing about it is the author's to choose, so it's the same for every program that declares the same things.

- **Buffers** follow WebGPU's rules for memory the host shares with the GPU, plus one rule of jess's: a struct or array member aligns to a multiple of 16 bytes, and a struct member's slot rounds up to one. That's what a uniform block demands, so every struct is valid in every buffer and one layout serves them all. Members lie largest alignment first, then in declaration order, so no member pads the next: a `Float` after two `Vec3f`s sits in the second one's padding.
- **Bindings:** each frequency is one bind group, pass 0, material 1, draw 2. Its uniform members are one block at binding 0; its resources follow in declaration order.
- **Vertex buffers:** per-vertex attributes are buffer 0 and per-instance attributes buffer 1, each packed in declaration order with no padding. Locations count across both, per-vertex first, and a matrix takes one per column.

`ProgramLayout` reports every offset, size, stride, binding, and location, so whatever writes a buffer for a program writes it from the layout rather than from the generated source.

## Lowering

One module writes all WGSL, with one naming scheme:

| What | WGSL name |
| --- | --- |
| uniform member `color` of the material | `u_material.f_color`, of struct `MaterialUniforms` |
| struct `Light`, declared third | `S2_Light`, its fields `f_color`, ... |
| texture and sampler `diffuse` of the material | `t_material_diffuse`, `s_material_diffuse` |
| attribute `uv` | `input.a_uv`; per instance, `input.i_uv` |
| matrix attribute `transform` at location 3 | its columns, `input.m3_transform` to `input.m6_transform` |
| varying `uv` | `input.v_uv` |
| built-in values | `vertex_index`, `instance_index`, and `front_facing`, entry point parameters only when the stage reads them; the fragment position is `input.position` |
| fragment output 0 | `color0` |
| temporaries | `e0`, `e1`, ... per entry point, in arena order |

Every non-leaf expression a stage reads becomes one `let`. The entry points are `vertex_main` and `fragment_main`. The same program always lowers to the same bytes.

Bindings and layouts belong to jess, never the author, and the lowering writes out what [Layout](#layout) decided: each frequency's block at binding 0 of its group, its textures and samplers after it, struct and block members in memory order, with `@align` and `@size` wherever jess's 16-byte rule differs from what WGSL would pick by itself. `WgslModule` carries the `ProgramLayout`, so the renderer never has to rediscover a layout from the source.

The fragment half of the textured, lit golden:

```wgsl
@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    let e0 = textureSample(t_material_diffuse, s_material_diffuse, input.v_uv);
    let e1 = normalize(input.v_normal);
    let e2 = dot(e1, u_material.f_lightDirection);
    let e3 = max(e2, 0.0);
    let e4 = e3 + u_material.f_ambient;
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
cargo test --test integration shader:: -- --nocapture
cargo test --test integration lua::shader::
```

Every lowered module in the tests goes through naga's parser and validator, and the uniform layouts jess reports are checked against the ones naga computes. The golden WGSL lives in `tests/shader/golden`; `JESS_BLESS=1` rewrites it. Only bless when every change to the output is intended, and say why in the commit. `lua_shader` builds a corpus of Luau shader modules (`tests/lua/shader/*.jsl`) through `jess.shader`, folds them, and lowers each to WGSL naga accepts, with the invalid ones failing where they should.

`tests/shader/triangle.rs` is the proof the layers compose: it builds a triangle program in the IR, lowers it, renders it headless with wgpu into an offscreen texture using only the bindings and locations `WgslModule` reported, and reads the center and corner pixels back. On a machine with no adapter at all, not even a software one, it prints `SKIPPED` and passes; `--nocapture` shows which adapter ran it.
