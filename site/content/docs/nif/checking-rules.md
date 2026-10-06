---
title: 'Check Rules'
description: 'Every rule jess nif check enforces, and the OpenMW loader code it follows.'
weight: 50
extra:
  kind: guide
---

`jess nif check` (and `jess.nif.check`, `checkBytes`, `nif:check()`) reads a NIF the way OpenMW does, then walks it with the rules of OpenMW's two loaders:

- **`components/nifosg/nifloader.cpp`**, the render loader: what draws, and how.
- **`components/nifbullet/bulletnifloader.cpp`**, the collision loader: what collides.

Each rule below names the OpenMW function it follows, and the source in `libs/nif/src/check.rs` and `libs/nif/src/check/gamebryo.rs` cites the same one. The reader-level rules follow `components/nif/niffile.cpp` (`Reader::parse`) and the records' own `read` and `post`. When OpenMW changes, these are the places to compare.

None of these rules are jess's opinion of what a good mesh looks like. Each one is something the engine does, found in the engine's code. If a rule here disagrees with OpenMW, the rule is wrong.

This page is the rule-by-rule reference. For running the check and reading its report, see [Checking and Fixing NIFs](@/docs/guides/nif-checking-and-fixing.md). For the flags, see [`jess nif`](@/docs/cli/nif.md).

## Severities

Every finding has one severity, from most to least severe:

| Severity | Means |
| --- | --- |
| `wontLoad` | OpenMW throws while loading the file and shows the error marker instead. `nif check` exits non-zero when any file has one. |
| `broken` | The file loads, but something in it is dropped or drawn differently than it was authored. |
| `missingAsset` | A file the NIF refers to doesn't exist in the load order. |
| `collision` | Collision differs from what the file looks like it asks for. |
| `performance` | Costs frame time or memory for no visible benefit. |
| `info` | Worth knowing, but not a problem by itself. |

The CLI shows `collision` and worse by default. `--show performance` or `--show info` goes further down, and `-v` shows everything, including files with no findings. In Luau the severities and codes are camelCase (`wontLoad`, `visibleEditorMarker`). In the CLI's YAML and JSON they're snake_case (`wont_load`, `visible_editor_marker`).

A file whose records can't be read gets only reader findings. Everything else needs the records, and there's no point guessing at a scene graph jess couldn't parse.

## Fixes

`jess nif fix` (`jess.nif.fix`, `nif:fix()`) repairs the findings that have exactly one right answer, on Morrowind NIFs:

| Fix | Repairs | What it does |
| --- | --- | --- |
| `rcn` | `stray_root_collision_node` (a `RootCollisionNode` nested under a child, with no `RCN`) | Adds `RCN` string data to the root, so OpenMW searches the whole tree for the collision node. |
| `ncc` | `lowercase_ncc` | Uppercases `ncc` to `NCC`, so the camera still collides. |
| `markers` | `visible_editor_marker` (Morrowind's `Tri EditorMarker` shapes) | Adds `MRK` string data to the root, so the markers don't draw in game. |
| `skinWeights` (`--only skin-weights`) | `skin_weight_out_of_range` | Drops weights for vertices the shape doesn't have. |
| `unreachable` | `unreachable_records` | Drops records nothing links to. |

Every fix keeps the model looking the way it does in Morrowind and makes OpenMW agree. Every other rule is either a judgment call or needs the mesh redone, and for those the [cookbook](@/docs/nif/optimization-cookbook.md) and [editing](@/docs/nif/editing.md) pages are the tools. `nif fix` doesn't guess.

`jess nif fix` backs up every file it replaces (`--no-backup` doesn't), and with `--output` writes into an override folder at the files' VFS paths instead, backing up whatever is already there. `--dry-run` reports what it would fix and writes nothing. With no inputs it fixes the whole load order, and refuses to do that in place: pass `--output` or `--dry-run`. `--data` adds folders or archives to look textures up in without fixing their NIFs. The [CLI reference](@/docs/cli/nif.md) has the rest.

## Reading the file

What makes OpenMW's `Nif::Reader` refuse a file, or skip part of it. These apply to every version.

| Code | Severity | Triggered by | OpenMW |
| --- | --- | --- | --- |
| `invalid_header` | wontLoad | The header can't be parsed: not a NIF at all, or a truncated or corrupt header. | `niffile.cpp` `Reader::parse` |
| `unknown_record_type` | wontLoad | A record type OpenMW's factory doesn't know. For files with a type table this is caught from the header alone, before any record is read. | `niffile.cpp` `Reader::parse` |
| `invalid_record_type_index` | wontLoad | A record's type index points past the header's type table. | `niffile.cpp` `Reader::parse` |
| `blank_record_type` | wontLoad | A record's type name is empty. | `niffile.cpp` `Reader::parse` |
| `link_out_of_range` | wontLoad | A link points past the last record. | `recordptr.hpp` `RecordPtrT::post` (`getRecord` is `mRecords.at`) |
| `wrong_link_type` | wontLoad | A link points to a record of the wrong class, like a shape whose data link is a node ("Failed to cast record pointer"). | `recordptr.hpp` `RecordPtrT::post` |
| `skin_missing_root_or_data` | wontLoad | An `NiSkinInstance` (or `BSSkin::Instance`) without its root or data. | `data.cpp` `NiSkinInstance::post` |
| `skin_bone_count_mismatch` | wontLoad | The skin instance and its data disagree on the bone count. | `data.cpp` `NiSkinInstance::post` |
| `skin_missing_bone` | wontLoad | A bone link is empty ("Oops: Missing bone!"). | `data.cpp` `NiSkinInstance::post` |
| `unreadable_record` | wontLoad | Any other read failure: truncated data, a bad count, a non-zero separator. The message says what and where. | the record's `read` |
| `unreadable_file` | wontLoad | The file itself couldn't be read from disk or the archive. | (jess) |
| `root_out_of_range` | broken | A root index doesn't point to a record. OpenMW warns and skips it. | `niffile.cpp` `Reader::parse` |
| `record_sizes_mismatch` | info | The header's record sizes don't add up to the file. OpenMW reads by type and doesn't use them, but jess can't check the roots from the header. | (header check) |

## The scene graph

| Code | Severity | Triggered by | OpenMW | Fix |
| --- | --- | --- | --- | --- |
| `no_root_nodes` | wontLoad | No root is an `NiAVObject`. Vanilla has two: `meshes/i/tx_crystal_02.nif` and `_03`. | `nifloader.cpp` `load` | |
| `root_transform_ignored` | info | Record 0 is a node other than `Bip01` with a non-identity transform. OpenMW discards it, as Morrowind does (Morrowind-era and Fallout 3+ files, not Oblivion's). | `nif/node.cpp` `NiNode::read` | |
| `visible_editor_marker` | broken | A `Tri EditorMarker` shape (Morrowind-era) or `EditorMarker`/`VisibilityEditorMarker` shape (later) draws, because the root has no `MRK` string data (or `BSXFlags` bit 32 later). | `nifloader.cpp` `handleNode` | `markers` (Morrowind-era) |
| `unsupported_particles` | broken | An `NiParticleSystem` or `BSStripParticleSystem`. Only Morrowind-style `NiParticles` draw. | `nifloader.cpp` `handleNode` | |
| `unhandled_billboard_mode` | broken | An `NiBillboardNode` mode other than always face camera, rotate about up, or rigid face camera. It falls back to rigid face camera. | `nifloader.cpp` `createNode` | |
| `empty_accumulator` | broken | An `NiSortAdjustNode` with no sub-sorter. It does nothing. | `nifloader.cpp` `handleNode` | |
| `sort_adjust_leaks` | info | An `NiSortAdjustNode` with a sorter. OpenMW applies it to every shape after it in the file, not just its children. | `nifloader.cpp` `handleNode` | |
| `effect_in_children` | info | A light or texture effect is a child but in no node's `effects` list, so it does nothing. | `nifloader.cpp` `handleNode` | |
| `light_ignored` | info | A light in an effects list. NIF lights don't light anything in OpenMW. | `nifloader.cpp` `handleEffect` | |
| `unhandled_texture_effect` | broken | An enabled `NiTextureEffect` that isn't a sphere-mapped environment map. | `nifloader.cpp` `handleEffect` | |
| `texture_effect_without_texture` | broken | An enabled environment map with no source texture. | `nifloader.cpp` `handleEffect` | |

Hidden subtrees (flag `0x1`) aren't drawn unless the hidden node itself has an `NiVisController`, so the draw rules skip them. Their controllers are still checked. Morrowind-era shapes named `shadow...` or `tri shadow...` aren't drawn either.

## Geometry

| Code | Severity | Triggered by | OpenMW | Fix |
| --- | --- | --- | --- | --- |
| `wrong_geometry_data` | broken | A shape whose data is the wrong kind (`NiTriStrips` with `NiTriShapeData`, say). It's silently dropped. | `nif/node.cpp` `NiGeometry::post` | |
| `empty_geometry` | info | A shape with no vertices or no triangles (no lines, for `NiLines`). It isn't drawn. | `nifloader.cpp` `handleNiGeometryData`, `handleBSGeometry` | |
| `vertex_index_out_of_range` | broken | A triangle uses a vertex the shape doesn't have. This may crash OpenMW. | `nifloader.cpp` `handleNiGeometryData` | |
| `uv_set_out_of_range` | broken | An enabled texture slot uses a UV set the geometry doesn't have. Set 0 is used instead. | `nifloader.cpp` `handleNiGeometryData` | |
| `skin_weight_out_of_range` | wontLoad | Skin data weights a vertex past the end of the shape (`influences.at(vertex)` throws). | `nifloader.cpp` `handleNiGeometry` | `skinWeights` |
| `unsupported_skinning` | broken | A `BSTriShape` skinned with anything but `BSSkinInstance` (Skyrim SE), or with a skin but no skin weights in its vertices. It's drawn unskinned. | `nifloader.cpp` `handleBSGeometry` | |

Skinned shapes use their skin partitions' triangles when they have any, as OpenMW does.

## Controllers

Only active controllers are checked. Inactive ones are skipped everywhere except in KF files.

| Code | Severity | Triggered by | OpenMW |
| --- | --- | --- | --- |
| `unhandled_controller` | broken | A controller type the object doesn't handle ("Unhandled controller"). Nodes take keyframe, path, visibility, roll, and look-at controllers. | `nifloader.cpp` `handleNodeControllers` |
| `controller_does_nothing` | broken | A geometry-only controller (`NiUVController`, `NiGeomMorpherController`) on something that isn't geometry, a particle controller on something that isn't particles, or any controller on a property that doesn't take them. | `handleNodeControllers`, `handleMeshControllers`, `handleProperty` |
| `controller_without_data` | broken / info | An `NiPathController` without path or percent data, an `NiLookAtController` without a target, an `NiGeomMorpherController` without morphs, or an `NiParticleColorModifier` without color data (broken). An `NiKeyframeController` without data or an interpolator (info). All are skipped. | `handleNodeControllers`, `handleNiGeometry`, `handleParticlePrograms` |
| `unsupported_interpolator` | broken | An interpolator of the wrong type, so the controller is skipped. Keyframe takes `NiTransformInterpolator`, visibility `NiBoolInterpolator`, roll, flip, and alpha `NiFloatInterpolator`, material color `NiPoint3Interpolator`. | `handleNodeControllers`, `handleTextureControllers`, `handleMaterialControllers` |
| `path_flags_ignored` | broken | An `NiPathController` with follow, bank, or flip flags. Only the position along the path is animated. | `nifosg/controller.cpp` `PathController` |
| `morpher_on_skinned_shape` | broken | An `NiGeomMorpherController` on a skinned shape. It's ignored. | `nifloader.cpp` `handleNiGeometry` |
| `morph_vertex_mismatch` | broken | The base morph's vertex count differs from the shape's. The morph is ignored. | `nifloader.cpp` `handleNiGeometry` |
| `unexpected_texture_controller` | broken | A controller other than `NiFlipController` on a texturing or shader property. | `nifloader.cpp` `handleTextureControllers` |
| `flip_controller_slot` | broken | An `NiFlipController` targeting a slot other than base. OpenMW always flips the base texture. | `nifloader.cpp` `handleTextureControllers` |
| `unexpected_material_controller` | broken | A controller other than `NiAlphaController` or `NiMaterialColorController` on a material. | `nifloader.cpp` `handleMaterialControllers` |
| `specular_color_controller` | broken | An `NiMaterialColorController` animating specular color in a Morrowind-era file. It's skipped. | `nifloader.cpp` `handleMaterialControllers` |

## Particles

| Code | Severity | Triggered by | OpenMW |
| --- | --- | --- | --- |
| `particles_without_controller` | broken | `NiParticles` with no particle system controller. It has no particles. | `nifloader.cpp` `handleParticleSystem` |
| `inactive_particles` | info | `NiParticles` whose only particle controllers are inactive. | `nifloader.cpp` `handleParticleSystem` |
| `particles_without_emitter` | broken | The particle controller has no emitter, so nothing is emitted. | `nifloader.cpp` `handleParticleSystem` |
| `unhandled_particle_modifier` | broken | A modifier other than grow/fade, gravity, bomb, color, or rotation, or a collider other than planar or spherical. | `nifloader.cpp` `handleParticlePrograms` |
| `particle_rotation_ignored` | info | `NiParticleRotation`. OpenMW ignores it (49 vanilla files). | `nifloader.cpp` `handleParticlePrograms` |

## Properties and textures

| Code | Severity | Triggered by | OpenMW |
| --- | --- | --- | --- |
| `unhandled_property` | broken | A property type OpenMW doesn't handle. Its effect isn't drawn. | `nifloader.cpp` `handleProperty` |
| `specular_ignored` | info | An enabled `NiSpecularProperty` in a Morrowind-era file. OpenMW never gives Morrowind NIFs specular highlights. | `nifloader.cpp` `applyDrawableProperties` |
| `unexpected_blend_mode` | broken | An `NiAlphaProperty` blend factor above 10 (not a blend mode). `SRC_ALPHA` is used instead. | `nifloader.cpp` `getBlendMode` |
| `destination_alpha_blend` | broken | The destination blend factor is `DST_ALPHA`, which OpenMW replaces with `ONE` to match D3D8. | `nifloader.cpp` `handleAlphaBlending` |
| `empty_base_texture` | broken | The base texture slot is enabled but has no source, and no controller fills it. | `nifloader.cpp` `handleTextureProperty` |
| `unhandled_texture_stage` | broken | An enabled `NiTexturingProperty` slot past the seven OpenMW binds (base, dark, detail, gloss, glow, bump, decal), or a `BSShaderTextureSet` slot past diffuse, normal, and glow. | `handleTextureProperty`, `handleTextureSet` |
| `unhandled_shader_type` | broken | A Bethesda shader type OpenMW doesn't implement. It's drawn with the default shader. | `nifloader.cpp` `getBSShaderPrefix`, `getBSLightingShaderPrefix` |
| `embedded_texture_format` | broken | An embedded `NiPixelData` format OpenMW misreads (its numbering of formats 4-9 differs from the file's) or can't read. | `nifloader.cpp` `handleInternalTexture` |
| `invalid_palette_texture` | broken | A palettized embedded texture without a palette, or not at 8 bits per pixel. | `nifloader.cpp` `handleInternalTexture` |
| `invalid_embedded_texture` | broken | An embedded texture with a zero-sized mipmap, or mipmap data past the end. It's ignored. | `nifloader.cpp` `handleInternalTexture` |

## Textures on disk

These come from resolving the NIF's texture paths the way OpenMW does, through the inputs, `--data`, and the load order. `nif:check()` and `checkBytes` resolve through the load order only, so with `openmw = false`, or no config to load, they have nothing to resolve against and skip these rules.

The report's summary says what was searched. `textures_checked` (`texturesChecked` in Luau) is whether textures were looked up at all. `load_order_checked` (`loadOrderChecked`) is whether the load order was among the places searched. When it's false (no config, or one that failed to load), textures still resolve against the inputs' own data folders and `--data`, but a texture installed by some other mod shows as `missing_texture`. It isn't missing. jess just wasn't told where to look.

| Code | Severity | Triggered by | OpenMW |
| --- | --- | --- | --- |
| `missing_texture` | missingAsset | No file for the path under `textures/` or `bookart/`, as `.dds` or as written. The message lists every path tried. | `misc/resourcehelpers.cpp` `correctTexturePath` |
| `uncompressed_texture` | performance | The texture resolves to something other than a `.dds`. It loads uncompressed and has its mipmaps built at load time. | `correctTexturePath` (prefers `.dds`) |

## Collision

Files up to 4.0.0.2 (and everything before 10.0.1.0) collide by `RootCollisionNode` and root string data. From 10.0.1.0 on, the root's `BSXFlags` decide, and Havok collision objects are ignored. `jess nif info` shows which setup a model ends up with (`collision.source`).

| Code | Severity | Triggered by | OpenMW | Fix |
| --- | --- | --- | --- | --- |
| `lowercase_ncc` | collision | Root string data like `ncc` or `Ncc`. Only an uppercase third `C` keeps camera collision, so any other case turns collision off entirely. The finding names the string data record, the one `nif fix` changes. | `bulletnifloader.cpp` `handleRoot` | `ncc` |
| `camera_only_collision` | info | The `RootCollisionNode` is empty, so only the camera collides, with the render geometry. | `bulletnifloader.cpp` `handleRoot` | |
| `stray_root_collision_node` | collision / broken | A `RootCollisionNode` OpenMW doesn't use. One nested below the root's children without `RCN` isn't found, so the render geometry collides too. An extra one beside the used one (only the last child is used) doesn't collide. In 10.0.1.0+ files, every one is stray. Before 10.0.1.0, it's broken rather than collision when it isn't hidden, because then it also draws. | `bulletnifloader.cpp` `handleNode`, `nif/node.cpp` `findRootCollisionNode` | `rcn` (nested, Morrowind-era) |
| `invisible_shape_collides` | collision | With collision built from render geometry, a shape that doesn't draw still collides: hidden, named as a shadow, under a non-root `Bounding Box` node, or a hidden editor marker. Not reported for skinned models, since actors collide as a box. | `bulletnifloader.cpp` `handleNode`, `handleGeometry` | |
| `switch_collision_first_child` | info | An `NiSwitchNode` or `NiFltAnimationNode` with several children, outside a collision node. Only the first collides. | `bulletnifloader.cpp` `handleNode` | |
| `invalid_bounding_box` | collision | The first `Bounding Box` node, which sets an actor's collision box, doesn't have a box bounding volume with positive extents. | `bulletnifloader.cpp` `findBoundingBox` | |
| `no_collision` | collision | 10.0.1.0+: the model has Havok collision, but the root's `BSXFlags` are missing or don't set bit 2, so it gets none. | `bulletnifloader.cpp` `handleRoot` | |
| `havok_ignored` | info | 10.0.1.0+: collision is built from render geometry and the Havok objects are ignored. | `bulletnifloader.cpp` `handleRoot` | |
| `shape_without_collision` | collision | 10.0.1.0+: a `BSTriShape`-family shape in a model that collides with render geometry. The collision loader only builds from `NiTriShape`/`NiTriStrips`, so it doesn't collide. | `bulletnifloader.cpp` `handleNode` (`getCollisionShape`) | |
| `render_geometry_collision` | performance | More than 5,000 collision triangles built from render geometry, because there's no `RootCollisionNode` (or, from 10.0.1.0, because Havok is ignored). This matters for statics. Actors collide as a box regardless. | `bulletnifloader.cpp` `handleRoot` | |

## Animations (.kf)

A file is checked as an animation when its name ends in `.kf`.

| Code | Severity | Triggered by | OpenMW |
| --- | --- | --- | --- |
| `kf_without_sequence` | broken | No `NiSequenceStreamHelper` root, so no animation loads. `NiControllerSequence` animations (later games) aren't read. | `nifloader.cpp` `loadKf` |
| `kf_without_text_keys` | broken | The helper's first extra data isn't an `NiTextKeyExtraData`, or it has none. The animation is discarded. | `nifloader.cpp` `loadKf` |
| `kf_unexpected_pair` | broken | An extra data and controller pair that isn't string data plus an `NiKeyframeController`. The pair is skipped. | `nifloader.cpp` `loadKf` |
| `kf_without_controllers` | broken | No usable keyframe controllers at all, so OpenMW discards the KF. | `nifloader.cpp` `loadKf` |

## Performance smells

| Code | Threshold | Triggered by | Fix |
| --- | --- | --- | --- |
| `unreachable_records` | any | Records no root reaches: read and allocated for nothing. Morrowind-era files only, since later files link records OpenMW reads but never resolves. | `unreachable` |
| `embedded_textures` | any | Textures stored in the file (`NiPixelData`). They can't be shared, compressed, or replaced like texture files. | |
| `many_shapes` | more than 100 | Rendered shapes: each is at least one draw call (before OpenMW's own merging). | |
| `many_triangles` | more than 50,000 | Rendered triangles in one model. | |
| `render_geometry_collision` | more than 5,000 | See [Collision](#collision). | |

Stats (`stats` in the report) count only what OpenMW would draw. Hidden subtrees, collision nodes, and filtered shapes are left out of `shapes`, `vertices`, and `triangles`. `collisionTriangles` counts what the collision loader builds from.

## Vanilla, for scale

Every NIF and KF in vanilla `Morrowind.bsa` (7,506 files), checked against the BSA's own textures: 2 won't load (`no_root_nodes`), 58 have a missing texture, 47 have invisible shapes that collide, 33 build heavy collision from render geometry, 17 have more than 100 shapes, and 137 have a root transform OpenMW ignores. If your mod's meshes do better than that, you're ahead of Bethesda. The bar is on the floor.
