//! Render graph registration, compilation, and the debug dump.

use std::path::PathBuf;

use jess::render::{
    AliasGroup, Attachment, ClearValue, CompiledGraph, Dependency, DependencyKind, GraphError, Lifetime, LoadOp, Pass,
    PassId, PassKind, PassMetadata, PassOutput, ReadUsage, RenderGraph, ResourceId, ResourceKind, StoreOp, TargetFormat,
    TargetSize,
};

const OWNER: &str = "@test/viewer";
const BLACK: LoadOp = LoadOp::Clear(ClearValue::Color([0.0, 0.0, 0.0, 1.0]));
const FAR: LoadOp = LoadOp::Clear(ClearValue::Depth(1.0));

fn mesh(name: &str) -> Pass {
    Pass::new(PassKind::Mesh, PassMetadata::new(name, OWNER))
}

fn fullscreen(name: &str) -> Pass {
    Pass::new(PassKind::Fullscreen, PassMetadata::new(name, OWNER))
}

fn store_color(resource: ResourceId) -> PassOutput {
    PassOutput::color(resource, BLACK, StoreOp::Store)
}

fn names(compiled: &CompiledGraph<'_>, passes: &[PassId]) -> Vec<String> {
    passes
        .iter()
        .map(|&pass| compiled.graph().pass(pass).unwrap().name().to_owned())
        .collect()
}

fn order(compiled: &CompiledGraph<'_>) -> Vec<String> {
    names(compiled, compiled.order())
}

fn resource_names(graph: &RenderGraph, group: &AliasGroup) -> Vec<String> {
    group
        .resources
        .iter()
        .map(|&resource| graph.resource(resource).unwrap().name.clone())
        .collect()
}

/// Compares against `tests/golden/<name>.txt`; `JESS_BLESS=1` rewrites the file instead.
fn assert_golden(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/render/golden")
        .join(format!("{name}.txt"));
    if std::env::var_os("JESS_BLESS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    assert!(actual == expected, "{} differs from the dump:\n{actual}", path.display());
}

/// The targets of the `mesh.standard` example.
struct Standard {
    graph: RenderGraph,
    depth: ResourceId,
    scene_color: ResourceId,
    selection_mask: ResourceId,
    output: ResourceId,
}

fn standard_targets() -> Standard {
    let mut graph = RenderGraph::new("mesh.standard");
    let depth = graph
        .add_depth_target("depth", TargetFormat::Depth32Float, TargetSize::VIEWPORT)
        .unwrap();
    let scene_color = graph
        .add_render_target("sceneColor", TargetFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    let selection_mask = graph
        .add_render_target("selectionMask", TargetFormat::R8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let output = graph
        .import_render_target("output", TargetFormat::Bgra8UnormSrgb, TargetSize::VIEWPORT)
        .unwrap();
    Standard {
        graph,
        depth,
        scene_color,
        selection_mask,
        output,
    }
}

fn standard_passes(targets: &Standard) -> Vec<Pass> {
    vec![
        mesh("depth").write(PassOutput::depth(targets.depth, FAR, StoreOp::Store)),
        mesh("opaque")
            .read(targets.depth, ReadUsage::DepthTest)
            .write(store_color(targets.scene_color)),
        mesh("selection")
            .read(targets.depth, ReadUsage::DepthTest)
            .write(store_color(targets.selection_mask)),
        fullscreen("outline")
            .read(targets.scene_color, ReadUsage::Texture)
            .read(targets.selection_mask, ReadUsage::Texture)
            .write(PassOutput::color(targets.output, LoadOp::DontCare, StoreOp::Store)),
    ]
}

fn standard_graph() -> Standard {
    let mut targets = standard_targets();
    for pass in standard_passes(&targets) {
        targets.graph.add_pass(pass).unwrap();
    }

    targets
}

/// An HDR scene target and the imported output, with no passes.
fn scene_targets(name: &str) -> (RenderGraph, ResourceId, ResourceId) {
    let mut graph = RenderGraph::new(name);
    let scene_color = graph
        .add_render_target("sceneColor", TargetFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    let output = graph
        .import_render_target("output", TargetFormat::Bgra8UnormSrgb, TargetSize::VIEWPORT)
        .unwrap();
    (graph, scene_color, output)
}

fn compile_error(graph: &RenderGraph) -> GraphError {
    graph.compile().unwrap_err()
}

#[test]
fn mesh_standard_compiles_in_dependency_order() {
    let standard = standard_graph();
    let compiled = standard.graph.compile().unwrap();
    assert_eq!(order(&compiled), ["depth", "opaque", "selection", "outline"]);
    assert_eq!(compiled.culled(), []);
}

#[test]
fn mesh_standard_dependencies_name_their_targets() {
    let standard = standard_graph();
    let compiled = standard.graph.compile().unwrap();
    let pass = |name| standard.graph.pass_id(name).unwrap();
    let read = |before, after, resource| Dependency {
        before: pass(before),
        after: pass(after),
        resource,
        kind: DependencyKind::ReadAfterWrite,
    };
    assert_eq!(
        compiled.dependencies(),
        [
            read("depth", "opaque", standard.depth),
            read("depth", "selection", standard.depth),
            read("opaque", "outline", standard.scene_color),
            read("selection", "outline", standard.selection_mask),
        ]
    );
}

#[test]
fn mesh_standard_lifetimes_and_aliasing() {
    let standard = standard_graph();
    let compiled = standard.graph.compile().unwrap();
    assert_eq!(compiled.lifetime(standard.depth), Some(Lifetime { first: 0, last: 2 }));
    assert_eq!(compiled.lifetime(standard.scene_color), Some(Lifetime { first: 1, last: 3 }));
    assert_eq!(
        compiled.lifetime(standard.selection_mask),
        Some(Lifetime { first: 2, last: 3 })
    );
    assert_eq!(compiled.lifetime(standard.output), Some(Lifetime { first: 3, last: 3 }));

    // Three formats, so three allocations; the imported output is never grouped.
    let groups: Vec<Vec<String>> = compiled
        .alias_groups()
        .iter()
        .map(|group| resource_names(&standard.graph, group))
        .collect();
    assert_eq!(groups, [["depth"], ["sceneColor"], ["selectionMask"]]);
}

#[test]
fn dependencies_fix_the_order_whatever_the_registration_order() {
    let mut targets = standard_targets();
    let mut passes = standard_passes(&targets);
    passes.reverse();
    for pass in passes {
        targets.graph.add_pass(pass).unwrap();
    }

    let compiled = targets.graph.compile().unwrap();
    // `selection` and `opaque` don't depend on each other, so registration order breaks the tie.
    assert_eq!(order(&compiled), ["depth", "selection", "opaque", "outline"]);
}

#[test]
fn independent_passes_keep_registration_order() {
    let mut graph = RenderGraph::new("independent");
    let output = graph
        .import_render_target("output", TargetFormat::Bgra8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let mut targets = Vec::new();
    for name in ["c", "a", "b"] {
        let target = graph
            .add_render_target(name, TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
            .unwrap();
        graph
            .add_pass(fullscreen(&format!("draw_{name}")).write(store_color(target)))
            .unwrap();
        targets.push(target);
    }

    let mut composite = fullscreen("composite").write(store_color(output));
    for &target in &targets {
        composite = composite.read(target, ReadUsage::Texture);
    }

    graph.add_pass(composite).unwrap();
    let compiled = graph.compile().unwrap();
    assert_eq!(order(&compiled), ["draw_c", "draw_a", "draw_b", "composite"]);
}

#[test]
fn writers_of_one_target_run_in_registration_order() {
    let mut targets = standard_targets();
    let depth = targets.depth;
    let scene_color = targets.scene_color;
    let output = targets.output;
    let graph = &mut targets.graph;
    graph
        .add_pass(mesh("depth").write(PassOutput::depth(depth, FAR, StoreOp::Store)))
        .unwrap();
    graph
        .add_pass(
            mesh("opaque")
                .read(depth, ReadUsage::DepthTest)
                .write(store_color(scene_color)),
        )
        .unwrap();
    graph
        .add_pass(mesh("transparent").read(depth, ReadUsage::DepthTest).write(PassOutput::color(
            scene_color,
            LoadOp::Load,
            StoreOp::Store,
        )))
        .unwrap();
    graph
        .add_pass(
            fullscreen("present")
                .read(scene_color, ReadUsage::Texture)
                .write(PassOutput::color(output, LoadOp::DontCare, StoreOp::Store)),
        )
        .unwrap();
    graph
        .add_pass(mesh("unusedMask").write(store_color(targets.selection_mask)))
        .unwrap();

    let compiled = graph.compile().unwrap();
    assert_eq!(order(&compiled), ["depth", "opaque", "transparent", "present"]);
    assert_eq!(names(&compiled, compiled.culled()), ["unusedMask"]);

    let transparent = graph.pass_id("transparent").unwrap();
    let into_transparent: Vec<(DependencyKind, ResourceId)> = compiled
        .dependencies()
        .iter()
        .filter(|dependency| dependency.after == transparent)
        .map(|dependency| (dependency.kind, dependency.resource))
        .collect();
    assert_eq!(
        into_transparent,
        [
            (DependencyKind::ReadAfterWrite, depth),
            (DependencyKind::LoadAfterWrite, scene_color)
        ]
    );

    // The final contents are what `present` reads, so it waits on the last writer only.
    let present = graph.pass_id("present").unwrap();
    let into_present: Vec<PassId> = compiled
        .dependencies()
        .iter()
        .filter(|dependency| dependency.after == present)
        .map(|dependency| dependency.before)
        .collect();
    assert_eq!(into_present, [transparent]);
}

#[test]
fn loading_before_the_first_write_depends_on_registration_order() {
    let (mut graph, scene_color, output) = scene_targets("loadFirst");
    graph
        .add_pass(mesh("transparent").write(PassOutput::color(scene_color, LoadOp::Load, StoreOp::Store)))
        .unwrap();
    graph.add_pass(mesh("opaque").write(store_color(scene_color))).unwrap();
    graph
        .add_pass(
            fullscreen("present")
                .read(scene_color, ReadUsage::Texture)
                .write(store_color(output)),
        )
        .unwrap();

    assert_eq!(
        compile_error(&graph),
        GraphError::LoadWithoutContents {
            pass: "transparent".to_owned(),
            resource: "sceneColor".to_owned()
        }
    );
}

#[test]
fn imported_targets_hold_contents_before_the_first_write() {
    let mut graph = RenderGraph::new("overlay");
    let output = graph
        .import_render_target("output", TargetFormat::Bgra8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let background = graph
        .import_render_target("background", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph
        .add_pass(
            fullscreen("overlay")
                .read(background, ReadUsage::Texture)
                .write(PassOutput::color(output, LoadOp::Load, StoreOp::Store)),
        )
        .unwrap();

    let compiled = graph.compile().unwrap();
    assert_eq!(order(&compiled), ["overlay"]);
    assert_eq!(compiled.alias_groups(), []);
}

#[test]
fn two_pass_cycles_are_rejected_by_name() {
    let mut graph = RenderGraph::new("pingPong");
    let output = graph
        .import_render_target("output", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let horizontal = graph
        .add_render_target("horizontal", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let vertical = graph
        .add_render_target("vertical", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph
        .add_pass(
            fullscreen("present")
                .read(vertical, ReadUsage::Texture)
                .write(store_color(output)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("blurHorizontal")
                .read(vertical, ReadUsage::Texture)
                .write(store_color(horizontal)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("blurVertical")
                .read(horizontal, ReadUsage::Texture)
                .write(store_color(vertical)),
        )
        .unwrap();

    let error = compile_error(&graph);
    assert_eq!(
        error,
        GraphError::Cycle {
            passes: vec!["blurHorizontal".into(), "blurVertical".into(), "blurHorizontal".into()]
        }
    );
    assert_eq!(
        error.to_string(),
        "dependency cycle: blurHorizontal -> blurVertical -> blurHorizontal"
    );
}

#[test]
fn longer_cycles_start_at_the_earliest_registered_pass() {
    let mut graph = RenderGraph::new("ring");
    let output = graph
        .import_render_target("output", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let first = graph
        .add_render_target("first", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let second = graph
        .add_render_target("second", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let third = graph
        .add_render_target("third", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph
        .add_pass(fullscreen("unrelated").write(PassOutput::color(output, LoadOp::DontCare, StoreOp::Store)))
        .unwrap();
    graph
        .add_pass(
            fullscreen("makeThird")
                .read(second, ReadUsage::Texture)
                .write(store_color(third)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("makeFirst")
                .read(third, ReadUsage::Texture)
                .write(store_color(first)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("makeSecond")
                .read(first, ReadUsage::Texture)
                .write(store_color(second)),
        )
        .unwrap();

    assert_eq!(
        compile_error(&graph),
        GraphError::Cycle {
            passes: ["makeThird", "makeFirst", "makeSecond", "makeThird"]
                .map(str::to_owned)
                .to_vec()
        }
    );
}

#[test]
fn writing_a_target_after_it_is_read_is_a_cycle() {
    // `tonemap` wants `sceneColor` before `grade` rewrites it, and `grade` needs `tonemap`'s
    // output. A read sees final contents, so the two wait on each other.
    let (mut graph, scene_color, output) = scene_targets("regrade");
    let tonemapped = graph
        .add_render_target("tonemapped", TargetFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    graph.add_pass(mesh("opaque").write(store_color(scene_color))).unwrap();
    graph
        .add_pass(
            fullscreen("tonemap")
                .read(scene_color, ReadUsage::Texture)
                .write(store_color(tonemapped)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("grade")
                .read(tonemapped, ReadUsage::Texture)
                .write(PassOutput::color(scene_color, LoadOp::Load, StoreOp::Store)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("present")
                .read(scene_color, ReadUsage::Texture)
                .write(store_color(output)),
        )
        .unwrap();

    assert_eq!(
        compile_error(&graph),
        GraphError::Cycle {
            passes: ["tonemap", "grade", "tonemap"].map(str::to_owned).to_vec()
        }
    );
}

#[test]
fn ids_from_another_graph_are_unknown() {
    let standard = standard_graph();
    let mut small = RenderGraph::new("small");
    let error = small
        .add_pass(mesh("opaque").write(store_color(standard.output)))
        .unwrap_err();
    assert_eq!(
        error,
        GraphError::UnknownResource {
            pass: "opaque".to_owned(),
            resource: standard.output
        }
    );
    assert_eq!(
        error.to_string(),
        "pass \"opaque\" uses resource #3, which this graph never registered"
    );
    assert_eq!(small.passes(), []);
}

#[test]
fn reading_a_transient_target_nobody_writes_is_rejected() {
    let mut targets = standard_targets();
    let graph = &mut targets.graph;
    graph
        .add_pass(mesh("depth").write(PassOutput::depth(targets.depth, FAR, StoreOp::Store)))
        .unwrap();
    graph
        .add_pass(
            mesh("opaque")
                .read(targets.depth, ReadUsage::DepthTest)
                .write(store_color(targets.scene_color)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("outline")
                .read(targets.scene_color, ReadUsage::Texture)
                .read(targets.selection_mask, ReadUsage::Texture)
                .write(store_color(targets.output)),
        )
        .unwrap();

    let error = compile_error(graph);
    assert_eq!(
        error,
        GraphError::ReadWithoutWriter {
            pass: "outline".to_owned(),
            resource: "selectionMask".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "pass \"outline\" reads \"selectionMask\", which no pass writes"
    );
}

#[test]
fn unused_transient_targets_are_rejected() {
    let mut targets = standard_targets();
    let graph = &mut targets.graph;
    graph
        .add_pass(mesh("depth").write(PassOutput::depth(targets.depth, FAR, StoreOp::Store)))
        .unwrap();
    graph
        .add_pass(
            mesh("opaque")
                .read(targets.depth, ReadUsage::DepthTest)
                .write(store_color(targets.output)),
        )
        .unwrap();
    assert_eq!(
        compile_error(graph),
        GraphError::UnusedResource {
            resource: "sceneColor".to_owned()
        }
    );
}

#[test]
fn reading_discarded_contents_is_rejected() {
    let mut targets = standard_targets();
    let graph = &mut targets.graph;
    graph
        .add_pass(mesh("depth").write(PassOutput::depth(targets.depth, FAR, StoreOp::Discard)))
        .unwrap();
    graph
        .add_pass(
            mesh("opaque")
                .read(targets.depth, ReadUsage::DepthTest)
                .write(store_color(targets.output)),
        )
        .unwrap();
    graph.add_pass(mesh("scene").write(store_color(targets.scene_color))).unwrap();
    graph
        .add_pass(mesh("selection").write(store_color(targets.selection_mask)))
        .unwrap();
    assert_eq!(
        compile_error(graph),
        GraphError::ReadDiscarded {
            pass: "opaque".to_owned(),
            resource: "depth".to_owned(),
            writer: "depth".to_owned()
        }
    );
}

#[test]
fn loading_discarded_contents_is_rejected() {
    let (mut graph, scene_color, output) = scene_targets("discardThenLoad");
    graph
        .add_pass(mesh("opaque").write(PassOutput::color(scene_color, BLACK, StoreOp::Discard)))
        .unwrap();
    graph
        .add_pass(mesh("transparent").write(PassOutput::color(scene_color, LoadOp::Load, StoreOp::Store)))
        .unwrap();
    graph
        .add_pass(
            fullscreen("present")
                .read(scene_color, ReadUsage::Texture)
                .write(store_color(output)),
        )
        .unwrap();
    assert_eq!(
        compile_error(&graph),
        GraphError::ReadDiscarded {
            pass: "transparent".to_owned(),
            resource: "sceneColor".to_owned(),
            writer: "opaque".to_owned()
        }
    );
}

#[test]
fn depth_used_only_inside_its_pass_may_be_discarded() {
    let mut targets = standard_targets();
    let graph = &mut targets.graph;
    graph
        .add_pass(mesh("opaque").write(store_color(targets.output)).write(PassOutput::depth(
            targets.depth,
            FAR,
            StoreOp::Discard,
        )))
        .unwrap();
    graph.add_pass(mesh("scene").write(store_color(targets.scene_color))).unwrap();
    graph
        .add_pass(mesh("selection").write(store_color(targets.selection_mask)))
        .unwrap();

    let compiled = graph.compile().unwrap();
    assert_eq!(order(&compiled), ["opaque"]);
    assert_eq!(names(&compiled, compiled.culled()), ["scene", "selection"]);
    assert_eq!(compiled.lifetime(targets.depth), Some(Lifetime { first: 0, last: 0 }));
    assert_eq!(compiled.lifetime(targets.scene_color), None);
}

#[test]
fn a_graph_that_reaches_no_imported_target_is_rejected() {
    let mut graph = RenderGraph::new("offscreen");
    let color = graph
        .add_render_target("color", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph.add_pass(mesh("draw").write(store_color(color))).unwrap();
    let error = compile_error(&graph);
    assert_eq!(
        error,
        GraphError::NoImportedOutput {
            graph: "offscreen".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "graph \"offscreen\" stores nothing into an imported target, so every pass would be culled"
    );
    assert_eq!(
        compile_error(&RenderGraph::new("empty")),
        GraphError::NoImportedOutput {
            graph: "empty".to_owned()
        }
    );
}

#[test]
fn overwritten_writes_are_culled_and_loaded_ones_kept() {
    let mut graph = RenderGraph::new("overwrite");
    let output = graph
        .import_render_target("output", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph.add_pass(fullscreen("background").write(store_color(output))).unwrap();
    graph.add_pass(fullscreen("scene").write(store_color(output))).unwrap();
    graph
        .add_pass(fullscreen("overlay").write(PassOutput::color(output, LoadOp::Load, StoreOp::Store)))
        .unwrap();

    let compiled = graph.compile().unwrap();
    assert_eq!(order(&compiled), ["scene", "overlay"]);
    assert_eq!(names(&compiled, compiled.culled()), ["background"]);
    assert_eq!(compiled.position(graph.pass_id("background").unwrap()), None);
    assert_eq!(compiled.position(graph.pass_id("overlay").unwrap()), Some(1));
    // The culled writer's ordering edge goes with it.
    assert_eq!(
        compiled.dependencies(),
        [Dependency {
            before: graph.pass_id("scene").unwrap(),
            after: graph.pass_id("overlay").unwrap(),
            resource: output,
            kind: DependencyKind::LoadAfterWrite,
        }]
    );
}

#[test]
fn culling_follows_contents_back_through_the_chain() {
    let mut graph = RenderGraph::new("culling");
    let output = graph
        .import_render_target("output", TargetFormat::Bgra8UnormSrgb, TargetSize::VIEWPORT)
        .unwrap();
    let scene = graph
        .add_render_target("scene", TargetFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    let normals = graph
        .add_render_target("normals", TargetFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    let normals_view = graph
        .add_render_target("normalsView", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph.add_pass(mesh("scene").write(store_color(scene))).unwrap();
    graph.add_pass(mesh("normals").write(store_color(normals))).unwrap();
    graph
        .add_pass(
            fullscreen("visualizeNormals")
                .read(normals, ReadUsage::Texture)
                .write(store_color(normals_view)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("present")
                .read(scene, ReadUsage::Texture)
                .write(store_color(output)),
        )
        .unwrap();

    let compiled = graph.compile().unwrap();
    assert_eq!(order(&compiled), ["scene", "present"]);
    assert_eq!(names(&compiled, compiled.culled()), ["normals", "visualizeNormals"]);
    assert_eq!(compiled.lifetime(normals), None);
    assert_eq!(compiled.lifetime(normals_view), None);
    assert_golden("culled", &compiled.to_string());
}

/// Bloom over an HDR scene: every intermediate is the same format and size, so the ones whose
/// lifetimes don't overlap share memory.
fn bloom_graph() -> RenderGraph {
    let half = TargetSize::Relative { scale: 0.5 };
    let mut graph = RenderGraph::new("bloom");
    let depth = graph
        .add_depth_target("depth", TargetFormat::Depth24Plus, TargetSize::VIEWPORT)
        .unwrap();
    let scene = graph
        .add_render_target("scene", TargetFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    let bright = graph.add_render_target("bright", TargetFormat::Rgba16Float, half).unwrap();
    let blur_horizontal = graph
        .add_render_target("blurHorizontal", TargetFormat::Rgba16Float, half)
        .unwrap();
    let blur_vertical = graph
        .add_render_target("blurVertical", TargetFormat::Rgba16Float, half)
        .unwrap();
    let combined = graph
        .add_render_target("combined", TargetFormat::Rgba16Float, TargetSize::VIEWPORT)
        .unwrap();
    let output = graph
        .import_render_target("output", TargetFormat::Bgra8UnormSrgb, TargetSize::VIEWPORT)
        .unwrap();

    graph
        .add_pass(
            mesh("opaque")
                .write(store_color(scene))
                .write(PassOutput::depth(depth, FAR, StoreOp::Discard)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("extractBright")
                .read(scene, ReadUsage::Texture)
                .write(store_color(bright)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("blurHorizontal")
                .read(bright, ReadUsage::Texture)
                .write(store_color(blur_horizontal)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("blurVertical")
                .read(blur_horizontal, ReadUsage::Texture)
                .write(store_color(blur_vertical)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("combine")
                .read(scene, ReadUsage::Texture)
                .read(blur_vertical, ReadUsage::Texture)
                .write(store_color(combined)),
        )
        .unwrap();
    graph
        .add_pass(
            fullscreen("tonemap")
                .read(combined, ReadUsage::Texture)
                .write(store_color(output)),
        )
        .unwrap();
    graph
}

#[test]
fn disjoint_lifetimes_of_one_descriptor_share_an_allocation() {
    let graph = bloom_graph();
    let compiled = graph.compile().unwrap();
    assert_eq!(
        order(&compiled),
        [
            "opaque",
            "extractBright",
            "blurHorizontal",
            "blurVertical",
            "combine",
            "tonemap"
        ]
    );

    let groups: Vec<Vec<String>> = compiled
        .alias_groups()
        .iter()
        .map(|group| resource_names(&graph, group))
        .collect();
    // `scene` lives until `combine`, so `combined` can't take its memory; `bright` is done
    // before `blurVertical` starts.
    assert_eq!(
        groups,
        [
            vec!["depth"],
            vec!["scene"],
            vec!["bright", "blurVertical"],
            vec!["blurHorizontal"],
            vec!["combined"]
        ]
    );
    assert_golden("bloom", &compiled.to_string());
}

#[test]
fn targets_used_by_the_same_pass_never_share() {
    let first = Lifetime { first: 1, last: 3 };
    assert!(first.overlaps(Lifetime { first: 3, last: 4 }));
    assert!(first.overlaps(Lifetime { first: 0, last: 1 }));
    assert!(first.overlaps(Lifetime { first: 2, last: 2 }));
    assert!(!first.overlaps(Lifetime { first: 4, last: 5 }));
    assert!(!first.overlaps(Lifetime { first: 0, last: 0 }));
}

#[test]
fn registration_rejects_malformed_targets() {
    let mut graph = RenderGraph::new("targets");
    graph
        .add_render_target("color", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    assert_eq!(
        graph.add_render_target("", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT),
        Err(GraphError::EmptyResourceName)
    );
    assert_eq!(
        graph.add_depth_target("color", TargetFormat::Depth32Float, TargetSize::VIEWPORT),
        Err(GraphError::DuplicateResource {
            resource: "color".to_owned()
        })
    );

    let error = graph
        .add_render_target("depthAsColor", TargetFormat::Depth32Float, TargetSize::VIEWPORT)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "render target \"depthAsColor\" can't have format Depth32Float"
    );
    assert_eq!(
        graph.import_depth_target("colorAsDepth", TargetFormat::R32Float, TargetSize::VIEWPORT),
        Err(GraphError::FormatKind {
            resource: "colorAsDepth".to_owned(),
            format: TargetFormat::R32Float,
            kind: ResourceKind::DepthTarget
        })
    );

    let empty = TargetSize::Absolute { width: 0, height: 512 };
    assert_eq!(
        graph.add_render_target("empty", TargetFormat::Rgba8Unorm, empty),
        Err(GraphError::InvalidSize {
            resource: "empty".to_owned(),
            size: empty
        })
    );
    assert!(
        graph
            .add_render_target("nan", TargetFormat::Rgba8Unorm, TargetSize::Relative { scale: f32::NAN })
            .is_err()
    );
    assert_eq!(graph.resources().len(), 1);
    assert_eq!(graph.resource_id("color").map(ResourceId::index), Some(0));
    assert_eq!(graph.resource_id("empty"), None);
}

#[test]
fn registration_rejects_malformed_passes() {
    let standard = standard_graph();
    let mut graph = standard.graph;
    let output = standard.output;

    assert_eq!(
        graph.add_pass(mesh("").write(store_color(output))),
        Err(GraphError::EmptyPassName)
    );
    let ownerless = Pass::new(PassKind::Mesh, PassMetadata::new("ownerless", "")).write(store_color(output));
    assert_eq!(
        graph.add_pass(ownerless),
        Err(GraphError::EmptyOwner {
            pass: "ownerless".to_owned()
        })
    );
    assert_eq!(
        graph.add_pass(mesh("opaque").write(store_color(output))),
        Err(GraphError::DuplicatePass {
            pass: "opaque".to_owned()
        })
    );
    assert_eq!(
        graph.add_pass(mesh("idle").read(standard.depth, ReadUsage::DepthTest)),
        Err(GraphError::NoOutputs { pass: "idle".to_owned() })
    );
    assert_eq!(graph.passes().len(), 4);
}

#[test]
fn reads_must_suit_the_target_kind() {
    let standard = standard_graph();
    let mut graph = standard.graph;

    let error = graph
        .add_pass(
            fullscreen("sampleDepth")
                .read(standard.depth, ReadUsage::Texture)
                .write(store_color(standard.output)),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "pass \"sampleDepth\" reads depth target \"depth\" as a color texture, which only a render target allows"
    );

    let error = graph
        .add_pass(
            mesh("testColor")
                .read(standard.scene_color, ReadUsage::DepthTest)
                .write(store_color(standard.output)),
        )
        .unwrap_err();
    assert_eq!(
        error,
        GraphError::IncompatibleRead {
            pass: "testColor".to_owned(),
            resource: "sceneColor".to_owned(),
            usage: ReadUsage::DepthTest,
            kind: ResourceKind::RenderTarget,
        }
    );

    assert!(matches!(
        graph.add_pass(
            fullscreen("sampleColorAsDepth")
                .read(standard.scene_color, ReadUsage::DepthTexture)
                .write(store_color(standard.output))
        ),
        Err(GraphError::IncompatibleRead {
            usage: ReadUsage::DepthTexture,
            ..
        })
    ));

    graph
        .add_pass(
            fullscreen("fog")
                .read(standard.depth, ReadUsage::DepthTexture)
                .write(store_color(standard.output)),
        )
        .unwrap();
}

#[test]
fn writes_must_suit_the_target_kind() {
    let standard = standard_graph();
    let mut graph = standard.graph;

    let error = graph.add_pass(mesh("depthIntoColor").write(PassOutput::depth(standard.scene_color, FAR, StoreOp::Store)));
    assert_eq!(
        error.unwrap_err().to_string(),
        "pass \"depthIntoColor\" writes render target \"sceneColor\" through a depth attachment, which only a depth target allows"
    );

    let error =
        graph.add_pass(mesh("colorIntoDepth").write(PassOutput::color(standard.depth, LoadOp::DontCare, StoreOp::Store)));
    assert!(matches!(
        error,
        Err(GraphError::IncompatibleWrite {
            attachment: Attachment::Color,
            ..
        })
    ));
}

#[test]
fn clear_values_must_fit_the_format() {
    let mut graph = RenderGraph::new("clears");
    let color = graph
        .add_render_target("color", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let picking = graph
        .add_render_target("picking", TargetFormat::R32Uint, TargetSize::VIEWPORT)
        .unwrap();
    let depth = graph
        .add_depth_target("depth", TargetFormat::Depth24Plus, TargetSize::VIEWPORT)
        .unwrap();

    let error = graph
        .add_pass(mesh("depthClear").write(PassOutput::color(color, FAR, StoreOp::Store)))
        .unwrap_err();
    assert_eq!(
        error,
        GraphError::ClearValueMismatch {
            pass: "depthClear".to_owned(),
            resource: "color".to_owned(),
            format: TargetFormat::Rgba8Unorm,
            clear: ClearValue::Depth(1.0),
        }
    );
    assert_eq!(
        error.to_string(),
        "pass \"depthClear\" clears \"color\" (Rgba8Unorm) with Depth(1.0)"
    );

    assert!(matches!(
        graph.add_pass(mesh("floatIds").write(PassOutput::color(picking, BLACK, StoreOp::Store))),
        Err(GraphError::ClearValueMismatch {
            format: TargetFormat::R32Uint,
            ..
        })
    ));
    assert!(matches!(
        graph.add_pass(mesh("idsOnColor").write(PassOutput::color(
            color,
            LoadOp::Clear(ClearValue::Uint([0; 4])),
            StoreOp::Store
        ))),
        Err(GraphError::ClearValueMismatch { .. })
    ));

    let too_far = LoadOp::Clear(ClearValue::Depth(1.5));
    assert_eq!(
        graph.add_pass(mesh("tooFar").write(PassOutput::depth(depth, too_far, StoreOp::Store))),
        Err(GraphError::InvalidClearValue {
            pass: "tooFar".to_owned(),
            resource: "depth".to_owned(),
            clear: ClearValue::Depth(1.5)
        })
    );
    let not_a_color = LoadOp::Clear(ClearValue::Color([f32::NAN, 0.0, 0.0, 1.0]));
    assert!(matches!(
        graph.add_pass(mesh("nanColor").write(PassOutput::color(color, not_a_color, StoreOp::Store))),
        Err(GraphError::InvalidClearValue { .. })
    ));

    graph
        .add_pass(mesh("pick").write(PassOutput::color(
            picking,
            LoadOp::Clear(ClearValue::Uint([0; 4])),
            StoreOp::Store,
        )))
        .unwrap();
}

#[test]
fn a_pass_uses_each_target_once() {
    let standard = standard_graph();
    let mut graph = standard.graph;

    let error = graph
        .add_pass(
            fullscreen("feedback")
                .read(standard.scene_color, ReadUsage::Texture)
                .write(store_color(standard.scene_color)),
        )
        .unwrap_err();
    assert_eq!(
        error,
        GraphError::ReadWriteConflict {
            pass: "feedback".to_owned(),
            resource: "sceneColor".to_owned()
        }
    );
    assert_eq!(error.to_string(), "pass \"feedback\" both reads and writes \"sceneColor\"");

    assert_eq!(
        graph.add_pass(
            mesh("depthTwice")
                .read(standard.depth, ReadUsage::DepthTest)
                .write(PassOutput::depth(standard.depth, LoadOp::Load, StoreOp::Store))
        ),
        Err(GraphError::ReadWriteConflict {
            pass: "depthTwice".to_owned(),
            resource: "depth".to_owned()
        })
    );
    assert_eq!(
        graph.add_pass(
            fullscreen("doubleWrite")
                .write(store_color(standard.output))
                .write(store_color(standard.output))
        ),
        Err(GraphError::DuplicateAccess {
            pass: "doubleWrite".to_owned(),
            resource: "output".to_owned()
        })
    );
    assert_eq!(
        graph.add_pass(
            fullscreen("doubleRead")
                .read(standard.scene_color, ReadUsage::Texture)
                .read(standard.scene_color, ReadUsage::Texture)
                .write(store_color(standard.output))
        ),
        Err(GraphError::DuplicateAccess {
            pass: "doubleRead".to_owned(),
            resource: "sceneColor".to_owned()
        })
    );
}

#[test]
fn attachments_must_fit_together() {
    let standard = standard_graph();
    let mut graph = standard.graph;
    let shadow = graph
        .add_depth_target("shadow", TargetFormat::Depth32Float, TargetSize::VIEWPORT)
        .unwrap();
    let small = graph
        .add_render_target(
            "small",
            TargetFormat::Rgba8Unorm,
            TargetSize::Absolute { width: 256, height: 256 },
        )
        .unwrap();

    assert_eq!(
        graph.add_pass(
            mesh("twoDepths")
                .read(standard.depth, ReadUsage::DepthTest)
                .write(PassOutput::depth(shadow, FAR, StoreOp::Store))
                .write(store_color(standard.output))
        ),
        Err(GraphError::MultipleDepthAttachments {
            pass: "twoDepths".to_owned(),
            first: "shadow".to_owned(),
            second: "depth".to_owned()
        })
    );
    assert_eq!(
        graph.add_pass(
            mesh("mixedSizes")
                .read(standard.depth, ReadUsage::DepthTest)
                .write(store_color(small))
        ),
        Err(GraphError::AttachmentSizeMismatch {
            pass: "mixedSizes".to_owned(),
            first: "small".to_owned(),
            second: "depth".to_owned()
        })
    );

    // Sampled targets aren't attachments, so their size is free.
    graph
        .add_pass(
            fullscreen("downsample")
                .read(standard.scene_color, ReadUsage::Texture)
                .write(store_color(small)),
        )
        .unwrap();
}

#[test]
fn metadata_carries_attribution() {
    let metadata = PassMetadata::new("outline", "@test/viewer").with_source("scripts/viewer.jsl", 42, 7);
    let source = metadata.source.as_ref().unwrap();
    assert_eq!(source.to_string(), "scripts/viewer.jsl:42:7");
    assert_eq!(metadata.owner, "@test/viewer");

    let mut graph = RenderGraph::new("attributed");
    let output = graph
        .import_render_target("output", TargetFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let id = graph
        .add_pass(Pass::new(PassKind::Fullscreen, metadata.clone()).write(store_color(output)))
        .unwrap();
    assert_eq!(graph.pass(id).unwrap().metadata, metadata);
    assert_eq!(id.to_string(), "pass #0");
}

#[test]
fn compiling_twice_gives_the_same_dump() {
    let graph = bloom_graph();
    assert_eq!(graph.compile().unwrap().to_string(), graph.compile().unwrap().to_string());
}
