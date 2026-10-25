//! Draw sets: retained draws checked when they are made, chosen by tag, shown and hidden, and
//! resized without being made again.

use std::sync::Arc;

use super::gpu;

use jess::gpu::{
    BindGroup, Binding, Buffer, BufferUsage, ColorState, DrawSet, DrawSetDesc, DrawSets, Gpu, GpuError, PipelineState,
    RenderPipeline, Target, TextureDesc, TextureUsage,
};
use jess::render::{ClearValue, TextureFormat};
use jess::shader::{DataType, Frequency, Place, PlaceRoot, ProgramBuilder, Rate, Step, Type};

const SIZE: u32 = 16;
const BLACK: [u8; 4] = [0, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

/// A pipeline drawing 2D positions in the material's color.
fn flat(gpu: &Gpu) -> Arc<RenderPipeline> {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec2f).unwrap();
    site.uniform(Frequency::Material, "color", DataType::Value(Type::Vec4f))
        .unwrap();
    let position = site.read_attribute(position);
    let zero = site.float(0.0).unwrap();
    let one = site.float(1.0).unwrap();
    let clip = site.construct(Type::Vec4f, &[position, zero, one]).unwrap();
    site.set_position(clip).unwrap();
    let place = Place::new(PlaceRoot::Uniform(Frequency::Material), &[Step::Field(0)]).unwrap();
    let color = site.load(place, &[]).unwrap();
    site.output(color).unwrap();
    let program = builder.finish().unwrap();
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    Arc::new(gpu.render_pipeline(&program, &state).unwrap())
}

/// The material group drawing in `color`.
fn material(gpu: &Gpu, pipeline: &RenderPipeline, color: [f32; 4]) -> Arc<BindGroup> {
    let uniforms = gpu.buffer(16, BufferUsage::UNIFORM).unwrap();
    gpu.write_buffer(&uniforms, 0, &bytes(&color)).unwrap();
    Arc::new(
        gpu.bind_group(pipeline, Frequency::Material, &[Binding::Uniform(&uniforms)])
            .unwrap(),
    )
}

/// A vertex buffer of `positions`.
fn vertices(gpu: &Gpu, positions: &[f32]) -> Arc<Buffer> {
    let bytes = bytes(positions);
    let buffer = gpu.buffer(bytes.len() as u64, BufferUsage::VERTEX).unwrap();
    gpu.write_buffer(&buffer, 0, &bytes).unwrap();
    Arc::new(buffer)
}

/// A triangle over the left half or the right half of the target, in `color`, tagged `tag`.
fn half(gpu: &Gpu, pipeline: &Arc<RenderPipeline>, right: bool, color: [f32; 4], tag: &str) -> Arc<DrawSet> {
    let (from, to) = if right { (0.0, 1.0) } else { (-1.0, 0.0) };
    let set = DrawSet::new(DrawSetDesc {
        pipeline: pipeline.clone(),
        material: Some(material(gpu, pipeline, color)),
        draw: None,
        vertices: Some(vertices(
            gpu,
            &[from, -1.0, to, -1.0, from, 1.0, to, -1.0, to, 1.0, from, 1.0],
        )),
        instances: None,
        indices: None,
        count: 6,
        instance_count: 1,
        tags: vec![tag.to_owned()],
    })
    .unwrap();
    Arc::new(set)
}

/// Draws every visible set of `sets` tagged `tag` over a black target, and returns three texels:
/// low in the left half, low in the right half, and high in the left half, toward the middle,
/// which only a half's second triangle covers.
fn drawn(gpu: &Gpu, sets: &DrawSets, tag: &str) -> ([u8; 4], [u8; 4], [u8; 4]) {
    let color = gpu
        .texture(&TextureDesc::d2(SIZE, SIZE, TextureFormat::Rgba8Unorm, TextureUsage::TARGET))
        .unwrap();
    let chosen = sets.matching(&[tag.to_owned()]);
    let draws: Vec<_> = chosen.iter().map(|set| set.draw(None)).collect();
    gpu.render(
        &[Target::cleared(&color, ClearValue::Color([0.0, 0.0, 0.0, 1.0]))],
        None,
        &draws,
    )
    .unwrap();
    let pixels = gpu.read_texture(&color, 0, 0).unwrap();
    let texel = |x: u32, y: u32| {
        let start = ((y * SIZE + x) * 4) as usize;
        [pixels[start], pixels[start + 1], pixels[start + 2], pixels[start + 3]]
    };
    let low = SIZE * 3 / 4;
    (
        texel(SIZE / 8, low),
        texel(SIZE / 2 + SIZE / 8, low),
        texel(SIZE * 3 / 8, SIZE / 8),
    )
}

#[test]
fn passes_choose_draw_sets_by_tag_and_visibility() {
    let Some(gpu) = gpu("passes_choose_draw_sets_by_tag_and_visibility") else {
        return;
    };
    let pipeline = flat(&gpu);
    let green = half(&gpu, &pipeline, false, [0.0, 1.0, 0.0, 1.0], "opaque");
    let red = half(&gpu, &pipeline, true, [1.0, 0.0, 0.0, 1.0], "overlay");
    let sets = DrawSets::new();
    sets.add(&green);
    sets.add(&red);
    sets.add(&green);
    assert_eq!(sets.len(), 2, "a set added twice is there once");

    assert_eq!(drawn(&gpu, &sets, "opaque"), (GREEN, BLACK, GREEN));
    assert_eq!(drawn(&gpu, &sets, "overlay"), (BLACK, [255, 0, 0, 255], BLACK));

    green.set_visible(false);
    assert!(!green.visible());
    assert_eq!(
        drawn(&gpu, &sets, "opaque"),
        (BLACK, BLACK, BLACK),
        "a hidden set isn't drawn"
    );
    green.set_visible(true);

    // Three vertices are the first of each half's two triangles: its lower left corner.
    green.set_count(3).unwrap();
    assert_eq!(green.count(), 3);
    assert_eq!(drawn(&gpu, &sets, "opaque"), (GREEN, BLACK, BLACK));

    let before = sets.version();
    assert!(sets.remove(&red));
    assert!(!sets.remove(&red), "a set is removed once");
    assert_eq!(sets.version(), before + 1);
    assert_eq!(drawn(&gpu, &sets, "overlay"), (BLACK, BLACK, BLACK));
}

#[test]
fn draw_sets_are_checked_as_draws_are() {
    let Some(gpu) = gpu("draw_sets_are_checked_as_draws_are") else {
        return;
    };
    let pipeline = flat(&gpu);
    let set = half(&gpu, &pipeline, false, [1.0; 4], "opaque");
    let invalid = |message: &str| GpuError::Invalid(format!("the draw set is invalid: {message}"));
    assert_eq!(
        set.set_count(7),
        Err(invalid("7 vertices of 8 bytes run past its 48-byte vertex buffer")),
    );
    assert_eq!(set.count(), 6, "a refused count changes nothing");
    assert_eq!(set.set_instance_count(0), Err(invalid("at least one instance")));

    let unbound = DrawSet::new(DrawSetDesc {
        pipeline,
        material: None,
        draw: None,
        vertices: Some(vertices(&gpu, &[0.0; 6])),
        instances: None,
        indices: None,
        count: 3,
        instance_count: 1,
        tags: Vec::new(),
    });
    assert_eq!(
        unbound.unwrap_err(),
        invalid("its program binds a material group, and the draw gives none")
    );
}
