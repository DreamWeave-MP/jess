//! Plans over levels, layers, and samples: a mip chain drawn level by level inside one target, a
//! multisampled target resolved into the host's, layers drawn one at a time and read as an array,
//! and copies of regions of levels of layers.

use std::sync::Arc;

use super::gpu;
use super::plans::covering;

use jess::gpu::{
    Binding, BufferUsage, ColorState, Gpu, Import, PipelineState, PlanBinding, PlanGroup, RenderPipeline, TextureDesc,
    TextureUsage, Work,
};
use jess::render::{
    ClearValue, CopyAt, CopyRegion, CopySize, Extent, LoadOp, Pass, PassKind, PassMetadata, PassOutput, ReadUsage,
    RenderGraph, StoreOp, TargetDesc, TargetSize, TextureFormat,
};
use jess::shader::{DataType, Frequency, Place, PlaceRoot, ProgramBuilder, ResourceId, ResourceKind, Step, Swizzle, Type};

const OWNER: &str = "@test/levels";
const COLOR: TextureFormat = TextureFormat::Rgba8Unorm;
const ROW: u64 = 256;
/// `ROW`, to index what a buffer reads back.
const ROW_BYTES: usize = 256;

fn pass(kind: PassKind, name: &str) -> Pass {
    Pass::new(kind, PassMetadata::new(name, OWNER))
}

fn square(side: u32) -> TargetSize {
    TargetSize::Absolute {
        width: side,
        height: side,
    }
}

fn state(multisampled: bool) -> PipelineState {
    PipelineState {
        colors: vec![ColorState::new(COLOR)],
        multisampled,
        ..PipelineState::default()
    }
}

/// A fullscreen pipeline filling its target with its material's color.
fn flat(gpu: &Gpu, multisampled: bool) -> Arc<RenderPipeline> {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    site.uniform(Frequency::Material, "color", DataType::Value(Type::Vec4f))
        .unwrap();
    let position = covering(&mut site);
    site.set_position(position).unwrap();
    let field = Place::new(PlaceRoot::Uniform(Frequency::Material), &[Step::Field(0)]).unwrap();
    let color = site.load(field, &[]).unwrap();
    site.output(color).unwrap();
    Arc::new(gpu.render_pipeline(&builder.finish().unwrap(), &state(multisampled)).unwrap())
}

/// A material group of `pipeline` holding `color`.
fn colored(gpu: &Gpu, pipeline: &RenderPipeline, color: [f32; 4]) -> PlanGroup {
    let buffer = gpu.buffer(16, BufferUsage::UNIFORM).unwrap();
    let bytes: Vec<u8> = color.iter().flat_map(|value| value.to_le_bytes()).collect();
    gpu.write_buffer(&buffer, 0, &bytes).unwrap();
    let group = gpu
        .bind_group(pipeline, Frequency::Material, &[Binding::Uniform(&buffer)])
        .unwrap();
    PlanGroup::Bound(Arc::new(group))
}

/// A fullscreen pipeline writing, at each pixel, half the texel of a material texture at twice
/// its coordinates: a level drawn from the one above it, darkened so each level shows which it
/// read. With the texture's declaration.
fn halve(gpu: &Gpu) -> (Arc<RenderPipeline>, ResourceId) {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let above = site.resource(Frequency::Material, "above", ResourceKind::Texture2D).unwrap();
    let position = covering(&mut site);
    site.set_position(position).unwrap();
    let pixel = site.builtin(jess::shader::Builtin::Position);
    let xy = site.swizzle(pixel, Swizzle::parse("xy").unwrap()).unwrap();
    let xy = site.convert(Type::Vec2u, xy).unwrap();
    let two = site.uint(2);
    let xy = site.multiply(xy, two).unwrap();
    let level = site.uint(0);
    let texel = site.load_texel(above, xy, None, level).unwrap();
    let half = site.float(0.5).unwrap();
    let darker = site.multiply(texel, half).unwrap();
    site.output(darker).unwrap();
    (
        Arc::new(gpu.render_pipeline(&builder.finish().unwrap(), &state(false)).unwrap()),
        above,
    )
}

#[test]
fn a_mip_chain_is_drawn_level_by_level_inside_one_target() {
    let Some(gpu) = gpu("a_mip_chain_is_drawn_level_by_level_inside_one_target") else {
        return;
    };
    let mut graph = RenderGraph::new("chain");
    let chain = graph.add_target("chain", TargetDesc::new(COLOR, square(16)).mips(3)).unwrap();
    let pixels = graph.import_buffer("pixels", ROW * 4).unwrap();
    let draw = |resource| PassOutput::color(resource, LoadOp::DontCare, StoreOp::Store);
    graph.add_pass(pass(PassKind::Fullscreen, "fill").write(draw(chain))).unwrap();
    for level in 1..3 {
        let down = pass(PassKind::Fullscreen, &format!("down{level}"))
            .read_at(chain, ReadUsage::Texture, level - 1, 0)
            .write(draw(chain).at(level, 0));
        graph.add_pass(down).unwrap();
    }
    graph
        .add_pass(
            pass(PassKind::Copy, "readBack")
                .read_at(chain, ReadUsage::Copy, 2, 0)
                .write(PassOutput::copy(pixels, LoadOp::DontCare, StoreOp::Store)),
        )
        .unwrap();

    let fill = flat(&gpu, false);
    let (down, above) = halve(&gpu);
    let from_above = Some(PlanGroup::Make(vec![PlanBinding::Target(above, "chain".to_owned())]));
    let work = vec![
        Work::Fullscreen {
            groups: [None, Some(colored(&gpu, &fill, [1.0, 0.0, 0.0, 1.0])), None],
            pipeline: fill.clone(),
        },
        Work::Fullscreen {
            pipeline: down.clone(),
            groups: [None, from_above.clone(), None],
        },
        Work::Fullscreen {
            pipeline: down,
            groups: [None, from_above, None],
        },
        Work::Copy,
    ];
    let plan = gpu.plan(&graph, work, Extent { width: 1, height: 1 }).unwrap();
    assert_eq!(plan.allocations(), 1, "every level is one texture's");
    let bytes = gpu.buffer(ROW * 4, BufferUsage::default()).unwrap();
    gpu.run(&plan, &jess::gpu::DrawSets::new(), &[Import::Buffer("pixels", &bytes)])
        .unwrap();

    // Level 2 is 4×4: a quarter of the red, each level having read the one above it.
    let pixels = gpu.read_buffer(&bytes, 0, ROW * 4).unwrap();
    let red = pixels[0];
    assert!((63..=65).contains(&red), "a quarter of 255, not {red}");
    assert_eq!(pixels[3 * ROW_BYTES + 12..3 * ROW_BYTES + 16], [red, 0, 0, 64]);
}

#[test]
fn a_multisampled_target_resolves_into_the_hosts() {
    let Some(gpu) = gpu("a_multisampled_target_resolves_into_the_hosts") else {
        return;
    };
    let mut graph = RenderGraph::new("msaa");
    let samples = graph
        .add_target("samples", TargetDesc::new(COLOR, TargetSize::VIEWPORT).samples(4))
        .unwrap();
    let screen = graph.import_render_target("screen", COLOR, TargetSize::VIEWPORT).unwrap();
    graph
        .add_pass(
            pass(PassKind::Fullscreen, "draw")
                .write(PassOutput::color(samples, LoadOp::DontCare, StoreOp::Discard).resolving(screen, 0, 0)),
        )
        .unwrap();
    let viewport = Extent { width: 8, height: 8 };
    let screen = gpu.texture(&TextureDesc::d2(8, 8, COLOR, TextureUsage::TARGET)).unwrap();

    let single = flat(&gpu, false);
    let work = |pipeline: &Arc<RenderPipeline>| {
        vec![Work::Fullscreen {
            groups: [None, Some(colored(&gpu, pipeline, [0.0, 1.0, 0.0, 1.0])), None],
            pipeline: pipeline.clone(),
        }]
    };
    assert_eq!(
        gpu.plan(&graph, work(&single), viewport).unwrap_err().to_string(),
        "pass draw is invalid: its pipeline draws 1 samples a texel, and the pass's targets have 4"
    );

    let multisampled = flat(&gpu, true);
    let plan = gpu.plan(&graph, work(&multisampled), viewport).unwrap();
    gpu.run(&plan, &jess::gpu::DrawSets::new(), &[Import::Texture("screen", &screen)])
        .unwrap();
    let pixels = gpu.read_texture(&screen, 0, 0).unwrap();
    assert_eq!(pixels[..4], [0, 255, 0, 255]);
    assert_eq!(pixels[pixels.len() - 4..], [0, 255, 0, 255]);
}

#[test]
fn layers_are_drawn_one_at_a_time_and_copied_by_region() {
    let Some(gpu) = gpu("layers_are_drawn_one_at_a_time_and_copied_by_region") else {
        return;
    };
    let mut graph = RenderGraph::new("layers");
    let atlas = graph
        .add_target("atlas", TargetDesc::new(COLOR, square(8)).layers(2))
        .unwrap();
    let pixels = graph.import_buffer("pixels", ROW * 6).unwrap();
    let paint = |layer| PassOutput::color(atlas, LoadOp::DontCare, StoreOp::Store).at(0, layer);
    graph.add_pass(pass(PassKind::Fullscreen, "first").write(paint(0))).unwrap();
    graph.add_pass(pass(PassKind::Fullscreen, "second").write(paint(1))).unwrap();
    graph
        .add_pass(
            pass(PassKind::Copy, "corner")
                .read_at(atlas, ReadUsage::Copy, 0, 1)
                .write(PassOutput::copy(pixels, LoadOp::DontCare, StoreOp::Store))
                .region(CopyRegion {
                    from: Some(CopyAt::Texel { x: 4, y: 4 }),
                    to: Some(CopyAt::Byte(ROW * 2)),
                    size: Some(CopySize::Texels { width: 2, height: 3 }),
                }),
        )
        .unwrap();
    let fill = flat(&gpu, false);
    let paint = |color| Work::Fullscreen {
        groups: [None, Some(colored(&gpu, &fill, color)), None],
        pipeline: fill.clone(),
    };
    let work = vec![paint([1.0, 0.0, 0.0, 1.0]), paint([0.0, 0.0, 1.0, 1.0]), Work::Copy];
    let plan = gpu.plan(&graph, work, Extent { width: 1, height: 1 }).unwrap();
    let bytes = gpu.buffer(ROW * 6, BufferUsage::default()).unwrap();
    gpu.write_buffer(&bytes, 0, &vec![7; ROW_BYTES * 6]).unwrap();
    gpu.run(&plan, &jess::gpu::DrawSets::new(), &[Import::Buffer("pixels", &bytes)])
        .unwrap();

    // Three rows of two blue texels from byte 512 on; nothing else touched.
    let pixels = gpu.read_buffer(&bytes, 0, ROW * 6).unwrap();
    let row = |index: usize| &pixels[ROW_BYTES * index..ROW_BYTES * index + 12];
    assert_eq!(row(1), [7; 12]);
    for index in 2..5 {
        assert_eq!(row(index), [0, 0, 255, 255, 0, 0, 255, 255, 7, 7, 7, 7], "row {index}");
    }
    assert_eq!(row(5), [7; 12]);
}

#[test]
fn a_layered_target_is_read_whole_as_an_array() {
    let Some(gpu) = gpu("a_layered_target_is_read_whole_as_an_array") else {
        return;
    };
    let mut graph = RenderGraph::new("array");
    let atlas = graph
        .add_target("atlas", TargetDesc::new(COLOR, square(4)).layers(3))
        .unwrap();
    let screen = graph.import_render_target("screen", COLOR, square(4)).unwrap();
    for layer in 0..3 {
        graph
            .add_pass(
                pass(PassKind::Fullscreen, &format!("layer{layer}"))
                    .write(PassOutput::color(atlas, LoadOp::DontCare, StoreOp::Store).at(0, layer)),
            )
            .unwrap();
    }
    graph
        .add_pass(
            pass(PassKind::Fullscreen, "pick")
                .read(atlas, ReadUsage::Texture)
                .write(PassOutput::color(
                    screen,
                    LoadOp::Clear(ClearValue::Color([0.0; 4])),
                    StoreOp::Store,
                )),
        )
        .unwrap();

    // Writes layer 2 of a material array texture at each pixel.
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let layers = site
        .resource(Frequency::Material, "layers", ResourceKind::Texture2DArray)
        .unwrap();
    let position = covering(&mut site);
    site.set_position(position).unwrap();
    let pixel = site.builtin(jess::shader::Builtin::Position);
    let xy = site.swizzle(pixel, Swizzle::parse("xy").unwrap()).unwrap();
    let xy = site.convert(Type::Vec2u, xy).unwrap();
    let layer = site.uint(2);
    let level = site.uint(0);
    let texel = site.load_texel(layers, xy, Some(layer), level).unwrap();
    site.output(texel).unwrap();
    let pick = Arc::new(gpu.render_pipeline(&builder.finish().unwrap(), &state(false)).unwrap());

    let fill = flat(&gpu, false);
    let paint = |color| Work::Fullscreen {
        groups: [None, Some(colored(&gpu, &fill, color)), None],
        pipeline: fill.clone(),
    };
    let work = vec![
        paint([1.0, 0.0, 0.0, 1.0]),
        paint([0.0, 1.0, 0.0, 1.0]),
        paint([0.0, 0.0, 1.0, 1.0]),
        Work::Fullscreen {
            groups: [
                None,
                Some(PlanGroup::Make(vec![PlanBinding::Target(layers, "atlas".to_owned())])),
                None,
            ],
            pipeline: pick,
        },
    ];
    let plan = gpu.plan(&graph, work, Extent { width: 1, height: 1 }).unwrap();
    let screen = gpu.texture(&TextureDesc::d2(4, 4, COLOR, TextureUsage::TARGET)).unwrap();
    gpu.run(&plan, &jess::gpu::DrawSets::new(), &[Import::Texture("screen", &screen)])
        .unwrap();
    assert_eq!(gpu.read_texture(&screen, 0, 0).unwrap()[..4], [0, 0, 255, 255]);
}
