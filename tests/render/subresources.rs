//! Levels, layers, and samples of targets: mip chains inside one target, multisampled targets
//! resolved, copy regions, and the dependencies each level of each layer carries on its own.

use jess::render::{
    ClearValue, CopyAt, CopyRegion, CopySize, DependencyKind, Extent, GraphError, LoadOp, Pass, PassKind, PassMetadata,
    PassOutput, ReadUsage, RenderGraph, ResolvedCopy, ResourceId, StoreOp, Subresource, TargetDesc, TargetSize, TexelAt,
    TextureFormat,
};

const OWNER: &str = "@test/levels";
const BLACK: LoadOp = LoadOp::Clear(ClearValue::Color([0.0, 0.0, 0.0, 1.0]));
const HDR: TextureFormat = TextureFormat::Rgba16Float;

fn pass(kind: PassKind, name: &str) -> Pass {
    Pass::new(kind, PassMetadata::new(name, OWNER))
}

fn fullscreen(name: &str) -> Pass {
    pass(PassKind::Fullscreen, name)
}

fn stored(resource: ResourceId) -> PassOutput {
    PassOutput::color(resource, LoadOp::DontCare, StoreOp::Store)
}

fn order(graph: &RenderGraph) -> Vec<String> {
    let compiled = graph.compile().unwrap();
    compiled
        .order()
        .iter()
        .map(|&pass| graph.pass(pass).unwrap().name().to_owned())
        .collect()
}

fn square(side: u32) -> TargetSize {
    TargetSize::Absolute {
        width: side,
        height: side,
    }
}

/// A bloom chain inside one target: the scene into level 0, each level drawn from the one above
/// it, and the whole chain read by the composite.
fn mip_chain() -> RenderGraph {
    let mut graph = RenderGraph::new("chain");
    let chain = graph.add_target("chain", TargetDesc::new(HDR, square(64)).mips(4)).unwrap();
    let screen = graph
        .import_render_target("screen", TextureFormat::Bgra8UnormSrgb, square(64))
        .unwrap();
    graph.add_pass(fullscreen("scene").write(stored(chain))).unwrap();
    for level in 1..4 {
        let down = fullscreen(&format!("down{level}"))
            .read_at(chain, ReadUsage::Texture, level - 1, 0)
            .write(stored(chain).at(level, 0));
        graph.add_pass(down).unwrap();
    }
    graph
        .add_pass(fullscreen("composite").read(chain, ReadUsage::Texture).write(stored(screen)))
        .unwrap();
    graph
}

#[test]
fn a_mip_chain_draws_each_level_from_the_one_above_it() {
    let graph = mip_chain();
    assert_eq!(order(&graph), ["scene", "down1", "down2", "down3", "composite"]);

    let compiled = graph.compile().unwrap();
    let composite = graph.pass_id("composite").unwrap();
    let feeding: Vec<&str> = compiled
        .dependencies()
        .iter()
        .filter(|dependency| dependency.after == composite)
        .map(|dependency| graph.pass(dependency.before).unwrap().name())
        .collect();
    // A whole read waits for the last writer of every level: one edge to each.
    assert_eq!(feeding, ["scene", "down1", "down2", "down3"]);
    assert!(
        compiled
            .dependencies()
            .iter()
            .all(|dependency| dependency.kind == DependencyKind::ReadAfterWrite)
    );
    assert_eq!(
        compiled.to_string(),
        "chain

scene
    writes: chain[level 0]

down1
    reads:  chain[level 0]
    writes: chain[level 1]

down2
    reads:  chain[level 1]
    writes: chain[level 2]

down3
    reads:  chain[level 2]
    writes: chain[level 3]

composite
    reads:  chain
    writes: screen

resources
    chain   Rgba16Float     64x64, 4 levels  transient  scene .. composite
    screen  Bgra8UnormSrgb  64x64            imported   composite

aliasing
    chain
"
    );
}

#[test]
fn a_whole_read_needs_every_level_written() {
    let mut graph = RenderGraph::new("gap");
    let chain = graph.add_target("chain", TargetDesc::new(HDR, square(8)).mips(3)).unwrap();
    let screen = graph.import_render_target("screen", HDR, square(8)).unwrap();
    graph.add_pass(fullscreen("scene").write(stored(chain))).unwrap();
    graph
        .add_pass(
            fullscreen("down")
                .read_at(chain, ReadUsage::Texture, 0, 0)
                .write(stored(chain).at(1, 0)),
        )
        .unwrap();
    graph
        .add_pass(fullscreen("composite").read(chain, ReadUsage::Texture).write(stored(screen)))
        .unwrap();
    let error = graph.compile().unwrap_err();
    assert_eq!(
        error,
        GraphError::ReadWithoutWriter {
            pass: "composite".to_owned(),
            resource: "chain".to_owned(),
            subresource: Some(Subresource::new(2, 0)),
        }
    );
    assert_eq!(
        error.to_string(),
        "pass \"composite\" reads level 2 of layer 0 of \"chain\", which no pass writes"
    );
}

#[test]
fn layers_are_written_and_read_on_their_own() {
    let mut graph = RenderGraph::new("faces");
    let faces = graph.add_target("faces", TargetDesc::new(HDR, square(16)).layers(6)).unwrap();
    let screen = graph.import_render_target("screen", HDR, square(16)).unwrap();
    for face in 0..6 {
        graph
            .add_pass(fullscreen(&format!("face{face}")).write(PassOutput::color(faces, BLACK, StoreOp::Store).at(0, face)))
            .unwrap();
    }
    // Reading face 2 waits for face 2 alone, so the passes drawing the rest are culled.
    graph
        .add_pass(
            fullscreen("show")
                .read_at(faces, ReadUsage::Texture, 0, 2)
                .write(stored(screen)),
        )
        .unwrap();
    let compiled = graph.compile().unwrap();
    let culled: Vec<&str> = compiled
        .culled()
        .iter()
        .map(|&pass| graph.pass(pass).unwrap().name())
        .collect();
    assert_eq!(culled, ["face0", "face1", "face3", "face4", "face5"]);
    assert_eq!(order(&graph), ["face2", "show"]);
}

#[test]
fn a_multisampled_target_resolves_into_a_single_sample_one() {
    let mut graph = RenderGraph::new("msaa");
    let samples = graph
        .add_target(
            "samples",
            TargetDesc::new(TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT).samples(4),
        )
        .unwrap();
    let depth = graph
        .add_target(
            "depth",
            TargetDesc::new(TextureFormat::Depth32Float, TargetSize::VIEWPORT).samples(4),
        )
        .unwrap();
    let screen = graph
        .import_render_target("screen", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph
        .add_pass(
            pass(PassKind::Mesh, "opaque")
                .write(PassOutput::color(samples, BLACK, StoreOp::Discard).resolving(screen, 0, 0))
                .write(PassOutput::depth(
                    depth,
                    LoadOp::Clear(ClearValue::Depth(1.0)),
                    StoreOp::Store,
                )),
        )
        .unwrap();
    graph
        .add_pass(
            pass(PassKind::Mesh, "overlay")
                .read(depth, ReadUsage::DepthTest)
                .write(PassOutput::color(samples, BLACK, StoreOp::Discard).resolving(screen, 0, 0)),
        )
        .unwrap();
    assert_eq!(order(&graph), ["opaque", "overlay"]);
    let dump = graph.compile().unwrap().to_string();
    assert!(dump.contains("    writes: samples -> screen, depth\n"), "{dump}");
    assert!(dump.contains("samples  Rgba8Unorm    viewport, 4 samples"), "{dump}");
}

#[test]
fn multisampled_targets_are_only_drawn_into_and_resolved() {
    let mut graph = RenderGraph::new("msaa");
    let samples = graph
        .add_target("samples", TargetDesc::new(TextureFormat::Rgba8Unorm, square(8)).samples(4))
        .unwrap();
    let single = graph
        .add_render_target("single", TextureFormat::Rgba8Unorm, square(8))
        .unwrap();
    let other = graph
        .add_render_target("other", TextureFormat::Rgba16Float, square(8))
        .unwrap();
    let small = graph
        .add_render_target("small", TextureFormat::Rgba8Unorm, square(4))
        .unwrap();
    let buffer = graph.add_buffer("bytes", 1024).unwrap();
    let resolve_error = |name: &str, output: PassOutput| {
        graph
            .clone()
            .add_pass(fullscreen(name).write(output))
            .unwrap_err()
            .to_string()
    };

    assert_eq!(
        graph
            .clone()
            .add_pass(fullscreen("sample").read(samples, ReadUsage::Texture).write(stored(single)))
            .unwrap_err()
            .to_string(),
        "pass \"sample\" uses multisampled \"samples\" as a color texture: a multisampled target is drawn into and resolved"
    );
    assert!(matches!(
        graph.clone().add_pass(
            pass(PassKind::Copy, "copy")
                .read(samples, ReadUsage::Copy)
                .write(PassOutput::copy(buffer, LoadOp::DontCare, StoreOp::Store))
        ),
        Err(GraphError::Multisampled { .. })
    ));
    assert_eq!(
        resolve_error("plain", stored(single).resolving(samples, 0, 0)),
        "pass \"plain\" can't resolve \"single\": it has one sample, so there is nothing to resolve"
    );
    assert_eq!(
        resolve_error("format", stored(samples).resolving(other, 0, 0)),
        "pass \"format\" can't resolve \"samples\": it is Rgba8Unorm and \"other\" is Rgba16Float"
    );
    assert_eq!(
        resolve_error("size", stored(samples).resolving(small, 0, 0)),
        "pass \"size\" can't resolve \"samples\": \"small\" is another size at level 0 of layer 0"
    );
    assert_eq!(
        resolve_error("buffer", stored(samples).resolving(buffer, 0, 0)),
        "pass \"buffer\" can't resolve \"samples\": \"bytes\" is a buffer"
    );
    assert_eq!(
        graph
            .clone()
            .add_pass(fullscreen("mixed").write(stored(samples)).write(stored(single)))
            .unwrap_err(),
        GraphError::SampleCountMismatch {
            pass: "mixed".to_owned(),
            first: "samples".to_owned(),
            second: "single".to_owned(),
        }
    );
    assert_eq!(
        graph
            .clone()
            .add_pass(
                fullscreen("twice")
                    .write(stored(samples).resolving(single, 0, 0))
                    .write(stored(single))
            )
            .unwrap_err(),
        GraphError::DuplicateAccess {
            pass: "twice".to_owned(),
            resource: "single".to_owned(),
        }
    );
}

#[test]
fn shapes_and_subresources_are_checked_as_they_are_registered() {
    let mut graph = RenderGraph::new("shapes");
    let shape_error = |target: TargetDesc| graph.clone().add_target("bad", target).unwrap_err().to_string();
    assert_eq!(
        shape_error(TargetDesc::new(HDR, square(8)).mips(5)),
        "target \"bad\" needs at most 4 mip levels at 8x8, not 5"
    );
    assert_eq!(
        shape_error(TargetDesc::new(HDR, square(8)).samples(2)),
        "target \"bad\" needs 1 or 4 samples a pixel, not 2"
    );
    assert_eq!(
        shape_error(TargetDesc::new(HDR, square(8)).samples(4).layers(2)),
        "target \"bad\" needs one level and one layer, being multisampled"
    );
    assert_eq!(
        shape_error(TargetDesc::new(HDR, square(8)).layers(0)),
        "target \"bad\" needs at least one mip level and one layer"
    );

    let chain = graph.add_target("chain", TargetDesc::new(HDR, square(8)).mips(2)).unwrap();
    let flat = graph.add_render_target("flat", HDR, square(4)).unwrap();
    assert_eq!(
        graph
            .clone()
            .add_pass(fullscreen("deep").write(stored(chain).at(2, 0)))
            .unwrap_err(),
        GraphError::SubresourceOutOfRange {
            pass: "deep".to_owned(),
            resource: "chain".to_owned(),
            subresource: Subresource::new(2, 0),
        }
    );
    assert_eq!(
        graph
            .clone()
            .add_pass(
                fullscreen("same")
                    .read_at(chain, ReadUsage::Texture, 1, 0)
                    .write(stored(chain).at(1, 0))
            )
            .unwrap_err(),
        GraphError::ReadWriteConflict {
            pass: "same".to_owned(),
            resource: "chain".to_owned(),
        }
    );
    assert_eq!(
        graph
            .clone()
            .add_pass(
                fullscreen("whole")
                    .read(chain, ReadUsage::Texture)
                    .write(stored(chain).at(1, 0))
            )
            .unwrap_err(),
        GraphError::ReadWriteConflict {
            pass: "whole".to_owned(),
            resource: "chain".to_owned(),
        }
    );
    // Level 1 of an 8×8 target is 4×4, the size of `flat`, so they draw together.
    graph
        .clone()
        .add_pass(fullscreen("together").write(stored(chain).at(1, 0)).write(stored(flat)))
        .unwrap();
    assert!(matches!(
        graph
            .clone()
            .add_pass(fullscreen("apart").write(stored(chain)).write(stored(flat))),
        Err(GraphError::AttachmentSizeMismatch { .. })
    ));
    assert_eq!(
        graph
            .clone()
            .add_pass(fullscreen("region").write(stored(flat)).region(CopyRegion {
                from: Some(CopyAt::Texel { x: 1, y: 1 }),
                ..CopyRegion::default()
            }))
            .unwrap_err(),
        GraphError::RegionOutsideCopy {
            pass: "region".to_owned()
        }
    );
}
/// An atlas of two levels of two layers, a tile, two buffers, and a depth target to copy between.
struct Regions {
    graph: RenderGraph,
    atlas: ResourceId,
    tile: ResourceId,
    bytes: ResourceId,
    small: ResourceId,
    depth: ResourceId,
}

fn regions() -> Regions {
    let mut graph = RenderGraph::new("regions");
    let atlas = graph
        .add_target(
            "atlas",
            TargetDesc::new(TextureFormat::Rgba8Unorm, square(32)).mips(2).layers(2),
        )
        .unwrap();
    let tile = graph.add_render_target("tile", TextureFormat::Rgba8Unorm, square(8)).unwrap();
    let bytes = graph.import_buffer("bytes", 4096).unwrap();
    let small = graph.add_buffer("small", 64).unwrap();
    let depth = graph
        .add_depth_target("depth", TextureFormat::Depth32Float, square(8))
        .unwrap();
    Regions {
        graph,
        atlas,
        tile,
        bytes,
        small,
        depth,
    }
}

/// A copy of `region` of level 1 of layer 1 of `from` into `to`.
fn copy(name: &str, from: ResourceId, to: ResourceId, region: CopyRegion) -> Pass {
    pass(PassKind::Copy, name)
        .read_at(from, ReadUsage::Copy, 1, 1)
        .write(PassOutput::copy(to, LoadOp::DontCare, StoreOp::Store))
        .region(region)
}

#[test]
fn copies_take_regions_of_levels_and_layers() {
    let Regions {
        graph,
        atlas,
        tile,
        bytes,
        ..
    } = regions();
    let resolve = |pass: &Pass| graph.resolve_copy(pass, pass.region, None);

    let into_tile = copy(
        "tile",
        atlas,
        tile,
        CopyRegion {
            from: Some(CopyAt::Texel { x: 8, y: 4 }),
            to: None,
            size: Some(CopySize::Texels { width: 8, height: 8 }),
        },
    );
    assert_eq!(
        resolve(&into_tile),
        Ok(Some(ResolvedCopy::Targets {
            from: TexelAt {
                subresource: Subresource::new(1, 1),
                x: 8,
                y: 4
            },
            to: TexelAt::default(),
            extent: Extent { width: 8, height: 8 },
        }))
    );
    let rows = copy(
        "rows",
        atlas,
        bytes,
        CopyRegion {
            from: Some(CopyAt::Texel { x: 0, y: 12 }),
            to: Some(CopyAt::Byte(1024)),
            size: None,
        },
    );
    // Level 1 is 16×16; from row 12 that is 16×4 texels, each row padded to 256 bytes.
    assert_eq!(
        resolve(&rows),
        Ok(Some(ResolvedCopy::TargetToBuffer {
            from: TexelAt {
                subresource: Subresource::new(1, 1),
                x: 0,
                y: 12
            },
            offset: 1024,
            row: 256,
            extent: Extent { width: 16, height: 4 },
        }))
    );
}

#[test]
fn copies_that_run_past_or_mix_their_units_are_refused() {
    let Regions {
        graph,
        atlas,
        tile,
        bytes,
        small,
        depth,
    } = regions();
    let refused = |pass: Pass| graph.clone().add_pass(pass).unwrap_err().to_string();
    assert_eq!(
        refused(copy(
            "past",
            atlas,
            tile,
            CopyRegion {
                from: Some(CopyAt::Texel { x: 12, y: 0 }),
                to: None,
                size: Some(CopySize::Texels { width: 8, height: 8 }),
            }
        )),
        "copy pass \"past\" is invalid: 8×8 texels at 12, 0 run past level 1 of layer 1 of \"atlas\", 16×16"
    );
    assert_eq!(
        refused(copy(
            "bytes",
            atlas,
            bytes,
            CopyRegion {
                size: Some(CopySize::Bytes(16)),
                ..CopyRegion::default()
            }
        )),
        "copy pass \"bytes\" is invalid: a copy with a target copies texels, not bytes"
    );
    assert_eq!(
        refused(
            pass(PassKind::Copy, "short")
                .read(small, ReadUsage::Copy)
                .write(PassOutput::copy(bytes, LoadOp::DontCare, StoreOp::Store))
                .region(CopyRegion {
                    from: Some(CopyAt::Byte(32)),
                    to: None,
                    size: Some(CopySize::Bytes(64)),
                })
        ),
        "copy pass \"short\" is invalid: \"small\" holds 64 bytes, and 64 from byte 32 run past it"
    );
    assert_eq!(
        refused(
            pass(PassKind::Copy, "depthPart")
                .read(depth, ReadUsage::Copy)
                .write(PassOutput::copy(bytes, LoadOp::DontCare, StoreOp::Store))
                .region(CopyRegion {
                    from: Some(CopyAt::Texel { x: 2, y: 2 }),
                    ..CopyRegion::default()
                })
        ),
        "copy pass \"depthPart\" is invalid: a copy with a Depth32Float target copies a whole level of a layer"
    );
}

#[test]
fn relative_targets_with_levels_never_resolve_smaller_than_their_levels_need() {
    let half = TargetDesc::new(HDR, TargetSize::Relative { scale: 0.5 }).mips(5);
    // Level 4 needs 16 pixels across level 0, whatever the viewport.
    assert_eq!(
        half.resolve(Extent { width: 10, height: 600 }),
        Extent { width: 16, height: 300 }
    );
    assert_eq!(
        half.extent(4, Some(Extent { width: 10, height: 10 })),
        Some(Extent { width: 1, height: 1 })
    );
    assert_eq!(
        TargetDesc::new(HDR, TargetSize::VIEWPORT).resolve(Extent { width: 10, height: 600 }),
        Extent { width: 10, height: 600 }
    );

    // So a level of it and a target of the same scale but fewer levels may differ in size, and
    // don't draw together.
    let mut graph = RenderGraph::new("floor");
    let chain = graph.add_target("chain", half).unwrap();
    let plain = graph
        .add_render_target("plain", HDR, TargetSize::Relative { scale: 0.5 })
        .unwrap();
    assert!(matches!(
        graph.add_pass(fullscreen("both").write(stored(chain)).write(stored(plain))),
        Err(GraphError::AttachmentSizeMismatch { .. })
    ));
}
