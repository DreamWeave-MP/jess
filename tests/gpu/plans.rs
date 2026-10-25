//! Plans: render graphs compiled for a device and run whole, with draw sets chosen by tag, depth
//! tested read-only, targets sampled by later passes, compute passes writing buffers, copies read
//! back, imported resources given to each run, and passes nothing observes culled.

use std::sync::Arc;

use super::gpu;

use jess::gpu::{
    Binding, Buffer, BufferUsage, ColorState, Compare, ComputePipeline, DepthState, DrawSet, DrawSetDesc, DrawSets, Gpu,
    Import, PipelineState, Plan, PlanBinding, PlanGroup, RenderPipeline, TextureDesc, TextureUsage, Work,
};
use jess::render::{
    ClearValue, Extent, LoadOp, Pass, PassKind, PassMetadata, PassOutput, ReadUsage, RenderGraph, StoreOp, TargetSize,
    TextureFormat,
};
use jess::shader::{
    Access, BinaryOperator, Builtin, DataType, Element, Frequency, Place, PlaceRoot, ProgramBuilder, Rate, ResourceId,
    ResourceKind, Site, Stage, Step, StorageId, Swizzle, Type, Value,
};

const SIZE: u32 = 16;
const VIEWPORT: Extent = Extent {
    width: SIZE,
    height: SIZE,
};
/// The bytes a row of a `SIZE`-wide 4-byte target takes in a buffer it is copied to.
const ROW: u32 = 256;
const OWNER: &str = "@test/plans";
const FAR: LoadOp = LoadOp::Clear(ClearValue::Depth(1.0));
const BLACK: LoadOp = LoadOp::Clear(ClearValue::Color([0.0, 0.0, 0.0, 1.0]));
const ZERO: LoadOp = LoadOp::Clear(ClearValue::Zero);

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

fn pass(kind: PassKind, name: &str) -> Pass {
    Pass::new(kind, PassMetadata::new(name, OWNER))
}

fn uniform(gpu: &Gpu, values: &[f32]) -> Buffer {
    let buffer = gpu.buffer(16, BufferUsage::UNIFORM).unwrap();
    gpu.write_buffer(&buffer, 0, &bytes(values)).unwrap();
    buffer
}

/// A target an imported `SIZE`×`SIZE` color target can be.
fn output(gpu: &Gpu) -> jess::gpu::Texture {
    gpu.texture(&TextureDesc::d2(SIZE, SIZE, TextureFormat::Rgba8Unorm, TextureUsage::TARGET))
        .unwrap()
}

/// The texel at `x`, `y` of a target copied into a buffer, rows `ROW` bytes apart.
fn copied(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let start = (y * ROW + x * 4) as usize;
    pixels[start..start + 4].try_into().unwrap()
}

/// Draws 3D positions in the material's color times the pass's tint, depth tested, writing depth
/// or not.
fn lit(gpu: &Gpu, write: bool) -> Arc<RenderPipeline> {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec3f).unwrap();
    site.uniform(Frequency::Pass, "tint", DataType::Value(Type::Vec4f)).unwrap();
    site.uniform(Frequency::Material, "color", DataType::Value(Type::Vec4f))
        .unwrap();
    let position = site.read_attribute(position);
    let one = site.float(1.0).unwrap();
    let clip = site.construct(Type::Vec4f, &[position, one]).unwrap();
    site.set_position(clip).unwrap();
    let field = |frequency| Place::new(PlaceRoot::Uniform(frequency), &[Step::Field(0)]).unwrap();
    let tint = site.load(field(Frequency::Pass), &[]).unwrap();
    let color = site.load(field(Frequency::Material), &[]).unwrap();
    let color = site.multiply(color, tint).unwrap();
    site.output(color).unwrap();
    let program = builder.finish().unwrap();
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        depth: Some(DepthState {
            format: TextureFormat::Depth32Float,
            compare: Compare::Less,
            write,
        }),
        ..PipelineState::default()
    };
    Arc::new(gpu.render_pipeline(&program, &state).unwrap())
}

/// A rectangle from `left` to `right` across the whole height, at `depth`, in `color`.
fn rectangle(
    gpu: &Gpu,
    pipeline: &Arc<RenderPipeline>,
    [left, right]: [f32; 2],
    depth: f32,
    color: [f32; 4],
    tag: &str,
) -> Arc<DrawSet> {
    let corners = [
        [left, -1.0],
        [right, -1.0],
        [left, 1.0],
        [right, -1.0],
        [right, 1.0],
        [left, 1.0],
    ];
    let vertices: Vec<f32> = corners.iter().flat_map(|[x, y]| [*x, *y, depth]).collect();
    let vertex_buffer = gpu.buffer(vertices.len() as u64 * 4, BufferUsage::VERTEX).unwrap();
    gpu.write_buffer(&vertex_buffer, 0, &bytes(&vertices)).unwrap();
    let material = gpu
        .bind_group(pipeline, Frequency::Material, &[Binding::Uniform(&uniform(gpu, &color))])
        .unwrap();
    let set = DrawSet::new(DrawSetDesc {
        pipeline: pipeline.clone(),
        material: Some(Arc::new(material)),
        draw: None,
        vertices: Some(Arc::new(vertex_buffer)),
        instances: None,
        indices: None,
        indirect: None,
        count: 6,
        instance_count: 1,
        tags: vec![tag.to_owned()],
    })
    .unwrap();
    Arc::new(set)
}

/// A triangle covering the target, from the vertex index alone: (-1, -1), (3, -1), (-1, 3).
fn covering(site: &mut Site<'_>) -> Value {
    let index = site.builtin(Builtin::VertexIndex);
    let one = site.uint(1);
    let two = site.uint(2);
    let shifted = site.binary(BinaryOperator::ShiftLeft, index, one).unwrap();
    let x = site.binary(BinaryOperator::BitAnd, shifted, two).unwrap();
    let y = site.binary(BinaryOperator::BitAnd, index, two).unwrap();
    let x = site.convert(Type::Float, x).unwrap();
    let y = site.convert(Type::Float, y).unwrap();
    let corner = site.construct(Type::Vec2f, &[x, y]).unwrap();
    let twice = site.float(2.0).unwrap();
    let corner = site.multiply(corner, twice).unwrap();
    let unit = site.float(1.0).unwrap();
    let corner = site.binary(BinaryOperator::Subtract, corner, unit).unwrap();
    let zero = site.float(0.0).unwrap();
    site.construct(Type::Vec4f, &[corner, zero, unit]).unwrap()
}

/// A fullscreen pipeline writing each pixel of a material texture with red and blue swapped; with
/// that texture's declaration.
fn swapped(gpu: &Gpu) -> (Arc<RenderPipeline>, ResourceId) {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let scene = site.resource(Frequency::Material, "scene", ResourceKind::Texture2D).unwrap();
    let position = covering(&mut site);
    site.set_position(position).unwrap();
    let pixel = site.builtin(Builtin::Position);
    let xy = site.swizzle(pixel, Swizzle::parse("xy").unwrap()).unwrap();
    let xy = site.convert(Type::Vec2u, xy).unwrap();
    let level = site.uint(0);
    let texel = site.load_texel(scene, xy, None, level).unwrap();
    let swapped = site.swizzle(texel, Swizzle::parse("zyxw").unwrap()).unwrap();
    site.output(swapped).unwrap();
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    (
        Arc::new(gpu.render_pipeline(&builder.finish().unwrap(), &state).unwrap()),
        scene,
    )
}

/// The scene: opaque sets drawn with depth, an overlay depth tested against it read-only, a
/// fullscreen pass sampling the result into the imported output, a copy of the output into an
/// imported buffer, and a debug pass nothing reads.
struct Scene {
    graph: RenderGraph,
    work: Vec<Work>,
    sets: DrawSets,
    /// What everything draws with, kept alive.
    _held: (Arc<RenderPipeline>, Arc<RenderPipeline>),
}

fn scene(gpu: &Gpu) -> Scene {
    let mut graph = RenderGraph::new("scene");
    let color = graph
        .add_render_target("color", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let depth = graph
        .add_depth_target("depth", TextureFormat::Depth32Float, TargetSize::VIEWPORT)
        .unwrap();
    let debug = graph
        .add_render_target("debug", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let out = graph
        .import_render_target("output", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let pixels = graph.import_buffer("pixels", u64::from(ROW * SIZE)).unwrap();
    for pass in [
        pass(PassKind::Mesh, "opaque")
            .write(PassOutput::color(color, BLACK, StoreOp::Store))
            .write(PassOutput::depth(depth, FAR, StoreOp::Store)),
        pass(PassKind::Mesh, "debug").write(PassOutput::color(debug, BLACK, StoreOp::Store)),
        pass(PassKind::Mesh, "overlay")
            .read(depth, ReadUsage::DepthTest)
            .write(PassOutput::color(color, LoadOp::Load, StoreOp::Store)),
        pass(PassKind::Fullscreen, "present")
            .read(color, ReadUsage::Texture)
            .write(PassOutput::color(out, LoadOp::DontCare, StoreOp::Store)),
        pass(PassKind::Copy, "readBack")
            .read(out, ReadUsage::Copy)
            .write(PassOutput::copy(pixels, LoadOp::DontCare, StoreOp::Store)),
    ] {
        graph.add_pass(pass).unwrap();
    }

    let solid = lit(gpu, true);
    let glaze = lit(gpu, false);
    let tint = gpu
        .bind_group(
            &solid,
            Frequency::Pass,
            &[Binding::Uniform(&uniform(gpu, &[0.6, 0.6, 0.6, 1.0]))],
        )
        .unwrap();
    let tint = Arc::new(tint);
    let sets = DrawSets::new();
    sets.add(&rectangle(gpu, &solid, [-1.0, 0.0], 0.25, [0.0, 1.0, 0.0, 1.0], "opaque"));
    sets.add(&rectangle(gpu, &solid, [-1.0, 1.0], 0.5, [1.0, 0.0, 0.0, 1.0], "opaque"));
    sets.add(&rectangle(gpu, &glaze, [-1.0, 1.0], 0.4, [0.0, 0.0, 1.0, 1.0], "overlay"));
    sets.add(&rectangle(gpu, &solid, [-1.0, 1.0], 0.1, [1.0, 1.0, 1.0, 1.0], "debug"));
    let (present, scene_texture) = swapped(gpu);
    let draw = |tag: &str| Work::Draw {
        tags: vec![tag.to_owned()],
        pipeline: None,
        pass: Some(PlanGroup::Bound(tint.clone())),
    };
    let work = vec![
        draw("opaque"),
        draw("debug"),
        draw("overlay"),
        Work::Fullscreen {
            pipeline: present.clone(),
            groups: [
                None,
                Some(PlanGroup::Make(vec![PlanBinding::Target(scene_texture, "color".to_owned())])),
                None,
            ],
        },
        Work::Copy,
    ];
    Scene {
        graph,
        work,
        sets,
        _held: (glaze, present),
    }
}

/// Runs `plan` into a fresh output and pixel buffer, and reads the pixels back.
fn run_scene(gpu: &Gpu, plan: &Plan, sets: &DrawSets) -> Vec<u8> {
    let target = output(gpu);
    let pixels = gpu.buffer(u64::from(ROW * SIZE), BufferUsage::default()).unwrap();
    gpu.run(
        plan,
        sets,
        &[Import::Texture("output", &target), Import::Buffer("pixels", &pixels)],
    )
    .unwrap();
    gpu.read_buffer(&pixels, 0, pixels.size()).unwrap()
}

#[test]
fn a_plan_draws_tests_depth_samples_and_reads_back() {
    let Some(gpu) = gpu("a_plan_draws_tests_depth_samples_and_reads_back") else {
        return;
    };
    let scene = scene(&gpu);
    let plan = gpu.plan(&scene.graph, scene.work.clone(), VIEWPORT).unwrap();
    assert_eq!(plan.order(), ["opaque", "overlay", "present", "readBack"], "debug is culled");
    assert_eq!(plan.allocations(), 2, "the color target and the depth target");

    // Green on the left in front of red; the overlay's blue only where red is, behind it; tinted
    // by 0.6, and then red and blue swapped.
    let pixels = run_scene(&gpu, &plan, &scene.sets);
    assert_eq!(copied(&pixels, 2, 8), [0, 153, 0, 255]);
    assert_eq!(copied(&pixels, SIZE - 2, 8), [153, 0, 0, 255]);

    // Each drawing pass is timed where the device can; a copy never is.
    let timings = gpu.timings(&plan).unwrap();
    if gpu.info().timestamps {
        let timings = timings.expect("a device with timestamps times its plans");
        let passes: Vec<&str> = timings.iter().map(|timing| timing.pass.as_str()).collect();
        assert_eq!(passes, plan.order());
        assert!(timings[..3].iter().all(|timing| timing.duration.is_some()));
        assert_eq!(timings[3].duration, None, "the copy isn't timed");
    } else {
        assert_eq!(timings, None);
    }

    // Draw sets change between runs; the plan doesn't.
    let hidden = scene.sets.matching(&["overlay".to_owned()]);
    hidden[0].set_visible(false);
    let pixels = run_scene(&gpu, &plan, &scene.sets);
    assert_eq!(copied(&pixels, SIZE - 2, 8), [0, 0, 153, 255], "the red behind it, swapped");
}

/// Adds 0.2 to every element of a pass storage buffer of four `Vec4f`s; with its declaration.
fn brighten(gpu: &Gpu) -> (Arc<ComputePipeline>, StorageId) {
    let mut builder = ProgramBuilder::compute([4, 1, 1]);
    let mut site = builder.site();
    let colors = site
        .storage(Frequency::Pass, "colors", Element::Value(Type::Vec4f), Access::ReadWrite)
        .unwrap();
    site.enter(Stage::Compute).unwrap();
    let invocation = site.builtin(Builtin::GlobalInvocationId);
    let index = site.swizzle(invocation, Swizzle::parse("x").unwrap()).unwrap();
    let at = Place::new(PlaceRoot::Storage(colors), &[Step::Index(index.expression())]).unwrap();
    let color = site.load(at, &[index]).unwrap();
    let step = site.float(0.2).unwrap();
    let brighter = site.add(color, step).unwrap();
    site.write(at, brighter, &[index]).unwrap();
    (Arc::new(gpu.compute_pipeline(&builder.finish().unwrap()).unwrap()), colors)
}

/// A fullscreen pipeline painting the first element of a material storage buffer; with its
/// declaration.
fn paint(gpu: &Gpu) -> (Arc<RenderPipeline>, StorageId) {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let colors = site
        .storage(Frequency::Material, "colors", Element::Value(Type::Vec4f), Access::Read)
        .unwrap();
    let position = covering(&mut site);
    site.set_position(position).unwrap();
    let first = site.uint(0);
    let at = Place::new(PlaceRoot::Storage(colors), &[Step::Index(first.expression())]).unwrap();
    let color = site.load(at, &[first]).unwrap();
    site.output(color).unwrap();
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    (
        Arc::new(gpu.render_pipeline(&builder.finish().unwrap(), &state).unwrap()),
        colors,
    )
}

/// Two compute passes brightening a transient buffer cleared to zero, the second loading the
/// first's work; a fullscreen pass painting from it; a copy of it into an imported buffer.
fn computed(gpu: &Gpu, workgroups: [u32; 3]) -> (RenderGraph, Vec<Work>) {
    let mut graph = RenderGraph::new("computed");
    let colors = graph.add_buffer("colors", 64).unwrap();
    let out = graph
        .import_render_target("output", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let kept = graph.import_buffer("kept", 64).unwrap();
    for pass in [
        pass(PassKind::Compute, "fill").write(PassOutput::storage(colors, ZERO, StoreOp::Store)),
        pass(PassKind::Compute, "again").write(PassOutput::storage(colors, LoadOp::Load, StoreOp::Store)),
        pass(PassKind::Fullscreen, "paint")
            .read(colors, ReadUsage::Buffer)
            .write(PassOutput::color(out, LoadOp::DontCare, StoreOp::Store)),
        pass(PassKind::Copy, "keep")
            .read(colors, ReadUsage::Copy)
            .write(PassOutput::copy(kept, LoadOp::DontCare, StoreOp::Store)),
    ] {
        graph.add_pass(pass).unwrap();
    }

    let (brighten, storage) = brighten(gpu);
    let (paint, read) = paint(gpu);
    let dispatch = || Work::Compute {
        pipeline: brighten.clone(),
        groups: [
            Some(PlanGroup::Make(vec![PlanBinding::Buffer(storage, "colors".to_owned())])),
            None,
            None,
        ],
        workgroups,
    };
    let work = vec![
        dispatch(),
        dispatch(),
        Work::Fullscreen {
            pipeline: paint,
            groups: [
                None,
                Some(PlanGroup::Make(vec![PlanBinding::Buffer(read, "colors".to_owned())])),
                None,
            ],
        },
        Work::Copy,
    ];
    (graph, work)
}

#[test]
fn compute_passes_write_buffers_later_passes_read_and_copy() {
    let Some(gpu) = gpu("compute_passes_write_buffers_later_passes_read_and_copy") else {
        return;
    };
    let (graph, work) = computed(&gpu, [1, 1, 1]);
    let plan = gpu.plan(&graph, work, VIEWPORT).unwrap();
    assert_eq!(plan.order(), ["fill", "again", "paint", "keep"]);
    let target = output(&gpu);
    let kept = gpu.buffer(64, BufferUsage::default()).unwrap();
    // Twice: the buffer is cleared at the start of every run, so the second run adds no more.
    for _ in 0..2 {
        gpu.run(
            &plan,
            &DrawSets::new(),
            &[Import::Texture("output", &target), Import::Buffer("kept", &kept)],
        )
        .unwrap();
        let values = floats(&gpu.read_buffer(&kept, 0, 64).unwrap());
        assert!(values.iter().all(|value| (value - 0.4).abs() < 1e-6), "{values:?}");
        let pixels = gpu.read_texture(&target, 0, 0).unwrap();
        assert_eq!(&pixels[..4], [102, 102, 102, 102]);
    }
}

/// A fullscreen pass filling a transient target, and one presenting it, swapped, to the output.
fn presented(gpu: &Gpu) -> (RenderGraph, Vec<Work>, Arc<RenderPipeline>, ResourceId) {
    let mut graph = RenderGraph::new("presented");
    let color = graph
        .add_render_target("color", TextureFormat::Rgba8Unorm, TargetSize::Relative { scale: 0.5 })
        .unwrap();
    let out = graph
        .import_render_target("output", TextureFormat::Rgba8Unorm, TargetSize::Relative { scale: 0.5 })
        .unwrap();
    graph
        .add_pass(pass(PassKind::Fullscreen, "fill").write(PassOutput::color(color, LoadOp::DontCare, StoreOp::Store)))
        .unwrap();
    graph
        .add_pass(
            pass(PassKind::Fullscreen, "present")
                .read(color, ReadUsage::Texture)
                .write(PassOutput::color(out, LoadOp::DontCare, StoreOp::Store)),
        )
        .unwrap();
    let (fill, colors) = paint(gpu);
    let buffer = gpu.buffer(64, BufferUsage::STORAGE).unwrap();
    gpu.write_buffer(&buffer, 0, &bytes(&[1.0, 0.0, 0.0, 1.0])).unwrap();
    let (present, scene) = swapped(gpu);
    let work = vec![
        Work::Fullscreen {
            pipeline: fill,
            groups: [
                None,
                Some(PlanGroup::Make(vec![PlanBinding::Storage(colors, Arc::new(buffer))])),
                None,
            ],
        },
        Work::Fullscreen {
            pipeline: present.clone(),
            groups: [
                None,
                Some(PlanGroup::Make(vec![PlanBinding::Target(scene, "color".to_owned())])),
                None,
            ],
        },
    ];
    (graph, work, present, scene)
}

#[test]
fn a_resized_plan_sizes_its_targets_again() {
    let Some(gpu) = gpu("a_resized_plan_sizes_its_targets_again") else {
        return;
    };
    let (graph, work, ..) = presented(&gpu);
    let plan = gpu.plan(&graph, work, VIEWPORT).unwrap();
    let small = plan.resized(&gpu, Extent { width: 8, height: 4 }).unwrap();
    assert_eq!(small.viewport(), Extent { width: 8, height: 4 });
    for (plan, width, height) in [(&plan, 8, 8), (&small, 4, 2)] {
        let target = gpu
            .texture(&TextureDesc::d2(
                width,
                height,
                TextureFormat::Rgba8Unorm,
                TextureUsage::TARGET,
            ))
            .unwrap();
        gpu.run(plan, &DrawSets::new(), &[Import::Texture("output", &target)])
            .unwrap();
        let pixels = gpu.read_texture(&target, 0, 0).unwrap();
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(pixels.chunks(4).all(|texel| texel == [0, 0, 255, 255]), "red, swapped");
    }
    let large = output(&gpu);
    assert_eq!(
        gpu.run(&small, &DrawSets::new(), &[Import::Texture("output", &large)])
            .unwrap_err()
            .to_string(),
        "the run gives output a 16×16 Rgba8Unorm texture: expected 4×2 Rgba8Unorm"
    );
}

/// The message of the error `result` is.
fn refused<T: std::fmt::Debug>(result: Result<T, jess::gpu::GpuError>) -> String {
    result.unwrap_err().to_string()
}

#[test]
fn plans_refuse_work_and_runs_that_dont_fit() {
    let Some(gpu) = gpu("plans_refuse_work_and_runs_that_dont_fit") else {
        return;
    };
    let (graph, work, present, scene) = presented(&gpu);
    assert_eq!(
        refused(gpu.plan(&graph, Vec::new(), VIEWPORT)),
        "the render graph has 2 passes, and the plan has work for 0: expected one each"
    );
    let mut copying = work.clone();
    copying[0] = Work::Copy;
    assert_eq!(
        refused(gpu.plan(&graph, copying, VIEWPORT)),
        "pass fill is invalid: it is a fullscreen pass, and its work copies"
    );
    let binding = |name: &str| Work::Fullscreen {
        pipeline: present.clone(),
        groups: [
            None,
            Some(PlanGroup::Make(vec![PlanBinding::Target(scene, name.to_owned())])),
            None,
        ],
    };
    let mut unread = work.clone();
    unread[0] = binding("color");
    assert_eq!(
        refused(gpu.plan(&graph, unread, VIEWPORT)),
        "pass fill is invalid: its material group binds color, which the pass doesn't declare as a texture or depth \
         texture read"
    );
    let mut unknown = work.clone();
    unknown[1] = binding("nothing");
    assert_eq!(
        refused(gpu.plan(&graph, unknown, VIEWPORT)),
        "pass present is invalid: its material group binds nothing, which the graph has no target or buffer called"
    );

    let plan = gpu.plan(&graph, work, VIEWPORT).unwrap();
    let target = gpu
        .texture(&TextureDesc::d2(8, 8, TextureFormat::Rgba8Unorm, TextureUsage::TARGET))
        .unwrap();
    let buffer = gpu.buffer(16, BufferUsage::default()).unwrap();
    let sets = DrawSets::new();
    for (imports, message) in [
        (Vec::new(), "pass present is invalid: the run gives nothing for output"),
        (
            vec![Import::Texture("output", &target), Import::Texture("output", &target)],
            "the run gives output twice",
        ),
        (
            vec![Import::Texture("elsewhere", &target)],
            "the run gives elsewhere, which the graph has no target or buffer called",
        ),
        (
            vec![Import::Texture("color", &target)],
            "the run gives color, which is the plan's own, not an imported one",
        ),
        (
            vec![Import::Buffer("output", &buffer)],
            "the run gives output a buffer, and it is a target",
        ),
    ] {
        assert_eq!(refused(gpu.run(&plan, &sets, &imports)), message);
    }
}

#[test]
fn dispatches_and_copies_are_refused_where_they_dont_fit() {
    let Some(gpu) = gpu("dispatches_and_copies_are_refused_where_they_dont_fit") else {
        return;
    };
    let (graph, work) = computed(&gpu, [0, 1, 1]);
    assert!(
        refused(gpu.plan(&graph, work, VIEWPORT))
            .starts_with("pass fill is invalid: a dispatch of 0×1×1 workgroups is invalid: expected from 1 to")
    );
    let (brighten, storage) = brighten(&gpu);
    let mut graph = RenderGraph::new("readOnly");
    let colors = graph.import_buffer("colors", 64).unwrap();
    let other = graph.import_buffer("other", 64).unwrap();
    graph
        .add_pass(
            pass(PassKind::Compute, "reads")
                .read(colors, ReadUsage::Buffer)
                .write(PassOutput::storage(other, LoadOp::DontCare, StoreOp::Store)),
        )
        .unwrap();
    let writes_read = Work::Compute {
        pipeline: brighten,
        groups: [
            Some(PlanGroup::Make(vec![PlanBinding::Buffer(storage, "colors".to_owned())])),
            None,
            None,
        ],
        workgroups: [1, 1, 1],
    };
    assert_eq!(
        refused(gpu.plan(&graph, vec![writes_read], VIEWPORT)),
        "pass reads is invalid: its pass group binds colors, which the pass doesn't declare as a storage write"
    );

    let mut graph = RenderGraph::new("cramped");
    let out = graph
        .import_render_target("output", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let pixels = graph.import_buffer("pixels", 64).unwrap();
    graph
        .add_pass(
            pass(PassKind::Copy, "readBack")
                .read(out, ReadUsage::Copy)
                .write(PassOutput::copy(pixels, LoadOp::DontCare, StoreOp::Store)),
        )
        .unwrap();
    assert_eq!(
        refused(gpu.plan(&graph, vec![Work::Copy], VIEWPORT)),
        "pass readBack is invalid: output at 16×16 takes 4096 bytes in a buffer, rows of 256, and pixels holds 64"
    );
}

#[test]
fn draw_passes_are_checked_against_their_sets() {
    let Some(gpu) = gpu("draw_passes_are_checked_against_their_sets") else {
        return;
    };
    let scene = scene(&gpu);
    let mut making = scene.work.clone();
    making[0] = Work::Draw {
        tags: vec!["opaque".to_owned()],
        pipeline: None,
        pass: Some(PlanGroup::Make(Vec::new())),
    };
    assert_eq!(
        refused(gpu.plan(&scene.graph, making, VIEWPORT)),
        "pass opaque is invalid: its work makes a pass group and names no pipeline to lay it out for"
    );

    // A set that writes depth, in a pass that only tests against it.
    let plan = gpu.plan(&scene.graph, scene.work.clone(), VIEWPORT).unwrap();
    let solid = lit(&gpu, true);
    scene
        .sets
        .add(&rectangle(&gpu, &solid, [-1.0, 1.0], 0.3, [1.0; 4], "overlay"));
    let target = output(&gpu);
    let pixels = gpu.buffer(u64::from(ROW * SIZE), BufferUsage::default()).unwrap();
    assert_eq!(
        refused(gpu.run(
            &plan,
            &scene.sets,
            &[Import::Texture("output", &target), Import::Buffer("pixels", &pixels)],
        )),
        "pass overlay is invalid: draw 1 is invalid: its pipeline writes depth, and the pass only tests against its \
         depth target"
    );

    // Without its pass group, a set's program misses one.
    let mut groupless = scene.work.clone();
    groupless[2] = Work::Draw {
        tags: vec!["overlay".to_owned()],
        pipeline: None,
        pass: None,
    };
    let plan = gpu.plan(&scene.graph, groupless, VIEWPORT).unwrap();
    let error = refused(gpu.run(
        &plan,
        &scene.sets,
        &[Import::Texture("output", &target), Import::Buffer("pixels", &pixels)],
    ));
    assert_eq!(
        error,
        "pass overlay is invalid: draw 0 is invalid: its program binds a pass group, and the draw gives none"
    );
}
