---
title: 'NIF Render Data'
description: 'What Morrowind''s NIFs actually use to draw, counted across Morrowind.bsa, and the NifScene translation that count decided.'
weight: 80
extra:
  kind: guide
---

NetImmerse defines stencil tests, fog, LOD nodes, bump maps, gloss maps, decal slots, and triangle strips. A viewer that supports all of that supports a lot of things Morrowind never shipped. So before `jess_nif::scene` existed, every NIF in `Morrowind.bsa` went through `NiStream` and got counted.

This page is that count, and what `NifScene` does with it.

## The Sample

The archive holds 7,343 NIFs and 163 KFs, Tribunal and Bloodmoon meshes included. Every one loads. The archive is read in place; nothing is extracted.

The scene-level half of the census is a test. Point it at an archive or a folder of meshes and it builds a `NifScene` for every file, checks every range and index, and prints what it found:

```sh
JESS_NIF_ARCHIVES="$HOME/Games/Morrowind/Data Files/Morrowind.bsa" \
    cargo test -p jess-nif --all-features --test nif scene::real_nifs -- --nocapture
```

Record-level counts come from walking `NiStream::objects`. `jess nif info` gives the same counts for one file.

## Representative Models

| What | Model | What's in it |
| --- | --- | --- |
| Static architecture | `x\ex_hlaalu_b_01.nif` | One `NiNode`, six `NiTriShape`s, five texturing properties sharing one material, a `RootCollisionNode` |
| Skinned creature | `r\guar.nif` | 69 nodes, most of them the `Bip01` skeleton, 17 skinned shapes, 50 keyframe controllers; its animation lives in `r\xguar.kf` |
| Light | `l\light_com_lantern_01.nif` | Nine shapes, a particle flame, an `AttachLight` node, a `Tri ShadowBox`. No `NiLight` record: color, radius, and flicker come from the plugin's `LIGH` record |
| Door | `d\ex_s_door.nif` | 20 shapes over 18 geometry records. The swing is the engine's, not the NIF's |
| Alpha-tested vegetation | `f\flora_bc_tree_02.nif`, `f\flora_tree_bm_01.nif` | 58 alpha properties on 61 shapes; Bloodmoon's trees test at 192 without sorting |
| Several texture slots | `wolf\head.nif`, `r\xdraugrlord.nif`, `inventory_window.nif` | Base plus glow, glow on UV set 1, base plus dark |
| Particles | `blightcloud.nif` | Seven `NiAutoNormalParticles` under `NiBSParticleNode`s, driven by particle controllers and gravity |
| Billboards | `e\frost_hit.nif` | Six `NiBillboardNode`s under keyframe and material color controllers |
| Environment map | `a\a_ebony_cuirass.nif` | Three skinned shapes and three `NiTextureEffect`s: the shine |
| Editor marker | `editormarker_box_01.nif` | `MRK` extra data on the root, a `Tri EditorMarker` shape |
| Switch or LOD node | none | There isn't one in the archive |

## Geometry

Every drawable shape is an `NiTriShape`: 41,986 of them, over 41,834 `NiTriShapeData` records. `NiTriStrips` doesn't occur. Neither does `NiLines`, `NiTriShapeDynamicData`, or `NiSkinPartition`.

Geometry sharing is real but rare: 82 data records are linked by more than one shape, mostly doors and Ashlander huts (`x\ex_s_foodhut.nif` links one record from eight shapes, `i\in_thirsk.nif` another from ten). No record anywhere has two parents in the scene graph, and there are no cycles.

| Vertex attributes | Shapes |
| --- | ---: |
| normals, colors, 1 UV set | 20,987 |
| normals, 1 UV set | 13,773 |
| normals, no UVs | 5,616 |
| normals, colors, no UVs | 969 |
| nothing but positions | 603 |
| normals, colors, 2 UV sets | 23 |
| normals, 2 UV sets | 15 |

The 603 bare shapes are the shadow meshes: 602 named `Tri Shadow...` and `w\shadowshield.nif`'s one. Vertex colors are on 52% of shapes, which is where Morrowind keeps its baked lighting. Shapes are small: 31,175 have at most 100 vertices, 400 have more than 1,000, and one (`x\bm_ex_iceberg.nif`) has more than 10,000. The format's vertex counts are 16-bit, so 16-bit indices always fit.

Skinning is in 561 files: 3,107 shapes, every creature, body part, and armor piece that bends. Skins have up to 70 bones. In 2,080 of the 3,107 skins every vertex follows one bone; 221 skins have a vertex with four influences, and 22 have one with five. A packed four-weights-per-vertex layout would already be lossy.

Particles are 682 shapes: 491 `NiRotatingParticles` and 191 `NiAutoNormalParticles`. Their geometry records hold particle start positions, not triangles. What gets drawn comes from the particle system controller, which is a simulation.

## Properties

| Property | Records | Notes |
| --- | ---: | --- |
| `NiTexturingProperty` | 25,871 | Nearly always on the shape; 14 on nodes. Apply mode `Modulate`, except 283 `Hilight`, mostly on body parts, clothing, candles, and torches |
| `NiSourceTexture` | 22,071 | All external files, all bare names with no folder: 15,057 `.tga`, 3,600 `.bmp`, 3,405 `.dds`, 8 `.nif`, 1 `.psd` |
| `NiMaterialProperty` | 19,498 | 414 with alpha below 1, 435 emissive, 7,025 with a specular color nobody sees |
| `NiAlphaProperty` | 4,605 | Eight flag combinations, below |
| `NiZBufferProperty` | 1,396 | Mostly on nodes: 1,073 test without writing, 323 both |
| `NiVertexColorProperty` | 556 | 546 on the roots of creatures and skeletons, turning vertex colors off; one emissive-only on `sky_clouds_01_no_tex.nif` |
| `NiWireframeProperty` | 15 | On the `Tri QuadPatch01` helper shapes of the base animation skeletons |
| `NiShadeProperty` | 10 | OpenMW ignores it |
| `NiSpecularProperty` | 8 | All enabled, in three files: a streetlight and two Bloodmoon wolf plaques |
| `NiDitherProperty` | 2 | OpenMW ignores it |
| `NiStencilProperty` | 0 | |
| `NiFogProperty` | 0 | |

Texture slots in use: base 25,866, dark 36, glow 20. Detail, gloss, bump, and decal slots: zero. The dark and glow slots sometimes sample UV set 1; everything else samples set 0. Filtering is `Trilerp` on all but 13 maps, and 272 base maps clamp in both directions instead of wrapping. Two `NiPixelData` records exist, as the roots of standalone texture NIFs; no `NiSourceTexture` embeds pixels.

The specular colors don't matter. OpenMW turns specular off for every Morrowind-version NIF (`applyDrawableProperties`: "Morrowind has its support disabled"), which is every NIF `NiStream` reads.

Alpha properties come in exactly eight flavors:

| Flags | Count | Meaning | Example |
| --- | ---: | --- | --- |
| `0x00ED` | 2,602 | Blend `SrcAlpha`, `InvSrcAlpha` | `blightcloud.nif` |
| `0x3AED` | 1,007 | Blend, test `>= 192`, no sorting | `f\flora_tree_bm_01.nif` |
| `0x000D` | 788 | Additive: `SrcAlpha`, `One` | `xwere_morph.nif` |
| `0x12ED` | 131 | Blend, test `> 100` | `wolf\xskin.1st.nif` |
| `0x10ED` | 68 | Blend; a test function set, testing off | `f\xbm_ice_wall.nif` |
| `0x110D` | 6 | `SrcAlpha`, `DstAlpha` | `i\act_rockslide01.nif` |
| `0x130D` | 2 | `SrcAlpha`, `DstAlpha`, test `> 0` | `l\light_logpile.nif` |
| `0x1001` | 1 | `One`, `One` | `e\magic_cast_frost.nif` |

Every one of them blends. Alpha testing without blending doesn't occur.

## Nodes, Names, and Flags

33,977 `NiNode`s, 3,399 `RootCollisionNode`s, 680 `NiBSParticleNode`s, 465 `NiBSAnimationNode`s, 184 `NiBillboardNode`s, 7 `AvoidNode`s, 5 `NiCamera`s. No switch, LOD, sort-adjust, or BSP nodes.

Roots are an `NiNode` in 6,900 files, a bare `NiTriShape` in 415 (cursors and menu parts), an `NiBSAnimationNode` in 25, and a camera, a billboard, or pixel data in the rest. The 163 KFs have an `NiSequenceStreamHelper` root and no scene at all. Graphs are shallow, at most 20 levels (bipeds), no rotation mirrors or scales, and the only non-unit scales are on ten billboards.

What decides visibility in OpenMW's `handleNode`:

- **The hidden flag (`0x1`)**: on 3,383 collision nodes, 1,134 shapes, 151 nodes, and 38 particle nodes.
- **`Tri Shadow` names**: 602 shapes, and every one is already hidden by a flag on itself or above it, so on retail content the name rule never decides anything by itself.
- **`Bounding Box` nodes**: 212, each with a bounding volume, none with geometry.
- **`MRK` extra data**: on the root of 10 editor marker files, each with `Tri EditorMarker` shapes.
- **Collision**: `NCO` extra data on 423 nodes, `NCC` on 50, `RCN` on 4.

## Controllers

| Controller | Records | On |
| --- | ---: | --- |
| `NiKeyframeController` | 6,164 in NIFs, 4,434 in KFs | Nodes, bones, billboards |
| `NiParticleSystemController` | 636 | Particles |
| `NiGeomMorpherController` | 326 | Shapes |
| `NiVisController` | 311 | Nodes and shapes |
| `NiAlphaController` | 188 | Material properties |
| `NiUVController` | 116 | Shapes |
| `NiPathController` | 66 | Billboards and nodes |
| `NiBSPArrayController` | 46 | Particles |
| `NiMaterialColorController` | 10 | Material properties |

No `NiFlipController`, `NiLightColorController`, `NiRollController`, or `NiLookAtController`. A NIF light that flickers is a plugin flag, not a controller.

## What NifScene Translates

`NifScene::from_stream(&NiStream)` walks the graph once, iteratively, and produces a scene a renderer can draw without knowing what an `NiTexturingProperty` is. The census decided the scope:

- **Nodes** in depth-first order, each with its local and world transform, its parent, where its subtree ends, and its visibility.
- **Drawables**, one per triangle shape, with a geometry, a material, a render layer, and a skin.
- **Geometry** as packed arrays in `GeometryBuffers`: positions, normals, colors, and UVs as flat `f32`s, triangle lists as `u16`, each geometry a range. One geometry per data record, so shared data is packed once. Strips would be unrolled into lists; retail has none, but `NiTriStripsData::triangles` already existed and mods use strips.
- **Materials** with the state OpenMW's `applyDrawableProperties` and `handleProperty` build: base, dark, and glow textures with their UV sets, apply mode, ambient, diffuse, emissive, and alpha after the vertex color and lighting rules, the vertex color mode, whether lights reach it, blending, alpha testing, depth test and write, wireframe, and culling. Properties inherit down the tree, nearest wins. Materials are deduplicated by the property records that make them.
- **Culling** comes from `NiStencilProperty`'s draw mode. Retail has no stencil properties, but mods use draw mode `Both` for double-sided foliage, and it's one enum. The stencil test itself isn't translated.
- **Skins**, because 561 files need them: each bone's scene node, its inverse bind transform, and its vertex weights packed per bone the way the NIF stores them, with no influence limit.
- **Render layers**: opaque, alpha-tested, and blended. A blended shape with the no-sorter flag, which is every Bloodmoon tree, draws with the opaque geometry in scene order, the way OpenMW's `handleAlphaBlending` does, so it lands in the alpha-test layer with its blend state intact.

The scene is the rest pose. Controllers aren't evaluated.

Texture paths stay exactly as the NIF stores them. Resolving them needs the VFS, which `jess-nif` doesn't have: `jess_assets::paths::texture_path` applies OpenMW's rules, prefixing `textures\` and preferring `.dds`, which is what turns 15,057 `.tga` names into the DDS files the archive actually contains.

## Nothing Disappears

Hidden, collision, shadow, bounding box, editor marker, and switched-off nodes are still nodes, and their shapes are still drawables. Each node's `visibility` says whether OpenMW draws it and, if not, why. A viewer that wants to show collision shows collision.

What the scene doesn't translate goes in `NifScene::issues`, once per record, with the record it came from:

- controllers (the rest pose is all there is);
- particle systems and lines;
- `NiTextureEffect`s and lights, whether in an effect list or in the graph (106 texture effects in retail: environment-mapped armor goes flat);
- detail, gloss, bump, and decal texture slots, embedded pixels, fog, and enabled stencil tests;
- LOD, animated switch, and sort-adjust nodes;
- roots that aren't scene objects, like a KF's.

Malformed files land there too. A link to a record that doesn't exist, a link to the wrong type, a triangle index past the vertices, attributes that don't match the vertex count, and a skin that doesn't fit its shape are reported and not drawn. A node that is its own ancestor is reported as a cycle and not followed. The only hard failure is a graph that expands past a million nodes, which takes a file whose levels each link the next one twice; real content stays in the hundreds.

On the archive above, every file builds a valid scene: 41,986 drawables (38,038 opaque, 1,007 alpha-tested, 2,941 blended; 33,572 visible, 7,353 collision, 1,051 hidden, 10 editor markers), 3,107 of them skinned.

## Source Identity

A viewer needs to go from a pixel to a record. Every node, geometry, material, texture, and skin carries the `NiKey` of the record it came from, and `NiLink::new(key).index()` turns that into the record index the NIF uses.

```text
picked drawable -> drawable.node -> nodes[node].source (the shape)
                                 -> parents, to the root (every node above it)
```

`NifScene::drawable_sources` walks that path. A record linked from two parents is two nodes with the same source, so a picked instance knows which parent it's under.

`NifScene`'s `Display` prints a deterministic dump of all of it: the node tree, every drawable with the records above it, geometry sizes, materials, textures, skins, and issues. The test assets' dumps are golden files in `libs/nif/tests/golden`; `JESS_BLESS=1` rewrites them.
