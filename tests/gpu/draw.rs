//! Drawing offscreen: pipelines from shader programs, bind groups checked against what the
//! programs declare, and passes read back pixel by pixel.

use super::gpu;

use jess::gpu::{
    AddressMode, BindGroup, Binding, BufferUsage, ColorState, Compare, DepthState, Draw, Filter, Gpu, GpuError,
    PipelineState, SamplerDesc, Target, Texture, TextureDesc, TextureUsage,
};
use jess::render::{ClearValue, TextureFormat};
use jess::shader::{
    BinaryOperator, Builtin, Frequency, Program, ProgramBuilder, Rate, ResourceKind, Site, Swizzle, Type, Value,
};

const SIZE: u32 = 64;
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

/// A `SIZE`×`SIZE` target of `format` that can be read back.
fn target(gpu: &Gpu, format: TextureFormat) -> Texture {
    gpu.texture(&TextureDesc::d2(SIZE, SIZE, format, TextureUsage::TARGET))
        .unwrap()
}

/// The texel at `x`, `y` of a read-back level of 4-byte texels.
fn texel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
    let start = ((y * SIZE + x) * 4) as usize;
    [pixels[start], pixels[start + 1], pixels[start + 2], pixels[start + 3]]
}

/// A triangle covering the target, from the vertex index alone: (-1, -1), (3, -1), (-1, 3).
fn covering(site: &mut Site<'_>, depth: f32) -> Value {
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
    let depth = site.float(depth).unwrap();
    site.construct(Type::Vec4f, &[corner, depth, unit]).unwrap()
}

/// Draws `pipeline` over a cleared target with one bind group, and reads it back.
fn draw_one(gpu: &Gpu, format: TextureFormat, clear: ClearValue, draw: Draw<'_>) -> Vec<u8> {
    let color = target(gpu, format);
    gpu.render(&[Target::cleared(&color, clear)], None, &[draw]).unwrap();
    gpu.read_texture(&color, 0, 0).unwrap()
}

fn instanced() -> Program {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec2f).unwrap();
    let transform = site.attribute(Rate::Instance, "transform", Type::Mat4f).unwrap();
    site.uniform(Frequency::Material, "color", jess::shader::DataType::Value(Type::Vec4f))
        .unwrap();
    let position = site.read_attribute(position);
    let transform = site.read_attribute(transform);
    let zero = site.float(0.0).unwrap();
    let one = site.float(1.0).unwrap();
    let local = site.construct(Type::Vec4f, &[position, zero, one]).unwrap();
    let clip = site.multiply(transform, local).unwrap();
    site.set_position(clip).unwrap();
    let place = jess::shader::Place::new(
        jess::shader::PlaceRoot::Uniform(Frequency::Material),
        &[jess::shader::Step::Field(0)],
    )
    .unwrap();
    let color = site.load(place, &[]).unwrap();
    site.output(color).unwrap();
    builder.finish().unwrap()
}

#[test]
fn instanced_triangles_draw_from_the_reported_layout() {
    let Some(gpu) = gpu("instanced_triangles_draw_from_the_reported_layout") else {
        return;
    };
    let program = instanced();
    let layout = program.layout();
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    let pipeline = gpu.render_pipeline(&program, &state).unwrap();

    let block = layout.group(Frequency::Material).uniform.as_ref().unwrap();
    let uniforms = gpu.buffer(u64::from(block.size), BufferUsage::UNIFORM).unwrap();
    gpu.write_buffer(&uniforms, u64::from(block.members[0].offset), &bytes(&[0.0, 1.0, 0.0, 1.0]))
        .unwrap();
    let material = gpu
        .bind_group(&pipeline, Frequency::Material, &[Binding::Uniform(&uniforms)])
        .unwrap();

    let [vertex, instance] = &layout.vertex_buffers;
    assert_eq!((vertex.stride, instance.stride), (8, 64));
    let vertices = gpu.buffer(24, BufferUsage::VERTEX).unwrap();
    gpu.write_buffer(&vertices, 0, &bytes(&[-0.5, -0.5, 0.5, -0.5, 0.0, 0.5]))
        .unwrap();
    // The triangle at half size, once left of the center and once right of it: column-major.
    let transforms: Vec<f32> = [-0.5f32, 0.5]
        .into_iter()
        .flat_map(|offset| {
            [
                0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, offset, 0.0, 0.0, 1.0,
            ]
        })
        .collect();
    let instances = gpu.buffer(128, BufferUsage::VERTEX).unwrap();
    gpu.write_buffer(&instances, 0, &bytes(&transforms)).unwrap();

    let draw = Draw {
        groups: [None, Some(&material), None],
        vertices: Some(&vertices),
        instances: Some(&instances),
        instance_count: 2,
        ..Draw::new(&pipeline, 3)
    };
    let pixels = draw_one(&gpu, TextureFormat::Rgba8Unorm, ClearValue::Color([0.0, 0.0, 1.0, 1.0]), draw);
    assert_eq!(texel(&pixels, SIZE / 4, SIZE / 2), GREEN, "the first instance is on the left");
    assert_eq!(
        texel(&pixels, SIZE * 3 / 4, SIZE / 2),
        GREEN,
        "the second instance is on the right"
    );
    assert_eq!(texel(&pixels, SIZE / 2, SIZE / 2), BLUE, "the instances are apart");
    assert_eq!(texel(&pixels, 0, 0), BLUE, "a corner is the clear color");
}

#[test]
fn a_texture_is_sampled_through_its_sampler() {
    let Some(gpu) = gpu("a_texture_is_sampled_through_its_sampler") else {
        return;
    };
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let texture = site
        .resource(Frequency::Material, "checker", ResourceKind::Texture2D)
        .unwrap();
    let sampler = site.resource(Frequency::Material, "checker", ResourceKind::Sampler).unwrap();
    let position = covering(&mut site, 0.5);
    site.set_position(position).unwrap();
    // The pixel's place in the target, from 0 to 1: the texture twice across it.
    let pixel = site.builtin(Builtin::Position);
    let xy = site.swizzle(pixel, Swizzle::parse("xy").unwrap()).unwrap();
    // Half the target's 64 pixels.
    let tile = site.float(32.0).unwrap();
    let uv = site.divide(xy, tile).unwrap();
    let color = site.sample(texture, sampler, uv, None).unwrap();
    site.output(color).unwrap();
    let program = builder.finish().unwrap();
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    let pipeline = gpu.render_pipeline(&program, &state).unwrap();

    let checker = gpu
        .texture(&TextureDesc::d2(2, 2, TextureFormat::Rgba8Unorm, TextureUsage::SAMPLED))
        .unwrap();
    let texels: Vec<u8> = [RED, GREEN, GREEN, RED].concat();
    gpu.write_texture(&checker, 0, 0, &texels).unwrap();
    let repeat = gpu
        .sampler(&SamplerDesc {
            filter: Filter::Nearest,
            address: AddressMode::Repeat,
            ..SamplerDesc::default()
        })
        .unwrap();
    let material = gpu
        .bind_group(
            &pipeline,
            Frequency::Material,
            &[Binding::Texture(texture, &checker), Binding::Sampler(sampler, &repeat)],
        )
        .unwrap();
    let draw = Draw {
        groups: [None, Some(&material), None],
        ..Draw::new(&pipeline, 3)
    };
    let pixels = draw_one(&gpu, TextureFormat::Rgba8Unorm, ClearValue::Color([0.0; 4]), draw);
    let quarter = SIZE / 4;
    for (x, y, expected) in [(0, 0, RED), (quarter, 0, GREEN), (0, quarter, GREEN), (quarter, quarter, RED)] {
        assert_eq!(texel(&pixels, x + 2, y + 2), expected, "at {x}, {y}");
    }
    assert_eq!(texel(&pixels, SIZE / 2 + 2, 2), RED, "the texture repeats");
}

/// A program drawing a covering triangle at `depth`, in the material's color.
fn flat(depth: f32) -> Program {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    site.uniform(Frequency::Material, "color", jess::shader::DataType::Value(Type::Vec4f))
        .unwrap();
    let position = covering(&mut site, depth);
    site.set_position(position).unwrap();
    let place = jess::shader::Place::new(
        jess::shader::PlaceRoot::Uniform(Frequency::Material),
        &[jess::shader::Step::Field(0)],
    )
    .unwrap();
    let color = site.load(place, &[]).unwrap();
    site.output(color).unwrap();
    builder.finish().unwrap()
}

fn colored(gpu: &Gpu, pipeline: &jess::gpu::RenderPipeline, color: [f32; 4]) -> (jess::gpu::Buffer, BindGroup) {
    let uniforms = gpu.buffer(16, BufferUsage::UNIFORM).unwrap();
    gpu.write_buffer(&uniforms, 0, &bytes(&color)).unwrap();
    let group = gpu
        .bind_group(pipeline, Frequency::Material, &[Binding::Uniform(&uniforms)])
        .unwrap();
    (uniforms, group)
}

#[test]
fn depth_keeps_the_nearest_whatever_the_order() {
    let Some(gpu) = gpu("depth_keeps_the_nearest_whatever_the_order") else {
        return;
    };
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        depth: Some(DepthState {
            format: TextureFormat::Depth32Float,
            compare: Compare::Less,
            write: true,
        }),
        ..PipelineState::default()
    };
    let near = gpu.render_pipeline(&flat(0.25), &state).unwrap();
    let far = gpu.render_pipeline(&flat(0.75), &state).unwrap();
    let (_red, red) = colored(&gpu, &near, [1.0, 0.0, 0.0, 1.0]);
    let (_green, green) = colored(&gpu, &far, [0.0, 1.0, 0.0, 1.0]);
    let near = Draw {
        groups: [None, Some(&red), None],
        ..Draw::new(&near, 3)
    };
    let far = Draw {
        groups: [None, Some(&green), None],
        ..Draw::new(&far, 3)
    };
    for order in [[near, far], [far, near]] {
        let color = target(&gpu, TextureFormat::Rgba8Unorm);
        let depth = target(&gpu, TextureFormat::Depth32Float);
        gpu.render(
            &[Target::cleared(&color, ClearValue::Color([0.0; 4]))],
            Some(&Target::cleared(&depth, ClearValue::Depth(1.0))),
            &order,
        )
        .unwrap();
        assert_eq!(texel(&gpu.read_texture(&color, 0, 0).unwrap(), 10, 10), RED);
        let depths = gpu.read_texture(&depth, 0, 0).unwrap();
        assert_eq!(f32::from_le_bytes(texel(&depths, 10, 10)), 0.25);
    }
}

#[test]
fn ids_are_drawn_to_an_integer_target() {
    let Some(gpu) = gpu("ids_are_drawn_to_an_integer_target") else {
        return;
    };
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    site.uniform(Frequency::Draw, "id", jess::shader::DataType::Value(Type::UInt))
        .unwrap();
    let position = covering(&mut site, 0.5);
    site.set_position(position).unwrap();
    let place = jess::shader::Place::new(
        jess::shader::PlaceRoot::Uniform(Frequency::Draw),
        &[jess::shader::Step::Field(0)],
    )
    .unwrap();
    let id = site.load(place, &[]).unwrap();
    site.output(id).unwrap();
    let program = builder.finish().unwrap();
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::R32Uint)],
        ..PipelineState::default()
    };
    let pipeline = gpu.render_pipeline(&program, &state).unwrap();
    let uniforms = gpu.buffer(16, BufferUsage::UNIFORM).unwrap();
    gpu.write_buffer(&uniforms, 0, &42u32.to_le_bytes()).unwrap();
    let group = gpu
        .bind_group(&pipeline, Frequency::Draw, &[Binding::Uniform(&uniforms)])
        .unwrap();
    let draw = Draw {
        groups: [None, None, Some(&group)],
        ..Draw::new(&pipeline, 3)
    };
    let ids = draw_one(&gpu, TextureFormat::R32Uint, ClearValue::Uint([0; 4]), draw);
    assert_eq!(u32::from_le_bytes(texel(&ids, 30, 40)), 42);
}

#[test]
fn what_doesnt_fit_is_refused() {
    let Some(gpu) = gpu("what_doesnt_fit_is_refused") else {
        return;
    };
    let program = flat(0.5);
    let floats = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    let pipeline = gpu.render_pipeline(&program, &floats).unwrap();
    let tiny = gpu.buffer(4, BufferUsage::UNIFORM).unwrap();
    let vertex = gpu.buffer(16, BufferUsage::VERTEX).unwrap();
    let sampled = gpu
        .texture(&TextureDesc::d2(4, 4, TextureFormat::Rgba8Unorm, TextureUsage::SAMPLED))
        .unwrap();
    let color = target(&gpu, TextureFormat::Rgba8Unorm);
    let (_buffer, group) = colored(&gpu, &pipeline, [1.0; 4]);

    let refusals = [
        (
            gpu.render_pipeline(
                &program,
                &PipelineState {
                    colors: vec![ColorState::new(TextureFormat::R32Uint)],
                    ..PipelineState::default()
                },
            )
            .unwrap_err(),
            "color output 0 is a Vec4f, which a R32Uint target can't hold: expected a UInt or UInt vector of 1 \
             channels or more",
        ),
        (
            gpu.render_pipeline(&program, &PipelineState::default()).unwrap_err(),
            "the program writes 1 color outputs, and the pipeline has 0 color targets: expected one per output",
        ),
        (
            gpu.bind_group(&pipeline, Frequency::Material, &[Binding::Uniform(&tiny)])
                .unwrap_err(),
            "the material bind group's uniform block is invalid: expected a uniform buffer of at least 16 bytes, \
             not a 4-byte uniform buffer",
        ),
        (
            gpu.bind_group(&pipeline, Frequency::Material, &[]).unwrap_err(),
            "the material bind group needs exactly one uniform block",
        ),
        (
            gpu.bind_group(&pipeline, Frequency::Pass, &[]).unwrap_err(),
            "the program binds nothing per pass: it needs no pass bind group",
        ),
        (
            gpu.render(
                &[Target::cleared(&color, ClearValue::Color([0.0; 4]))],
                None,
                &[Draw::new(&pipeline, 3)],
            )
            .unwrap_err(),
            "draw 0 is invalid: its program binds a material group, and the draw gives none",
        ),
        (
            gpu.render(
                &[Target::cleared(&color, ClearValue::Color([0.0; 4]))],
                None,
                &[Draw {
                    groups: [None, Some(&group), None],
                    vertices: Some(&vertex),
                    ..Draw::new(&pipeline, 3)
                }],
            )
            .unwrap_err(),
            "draw 0 is invalid: its program has no vertex attributes, and the draw gives a vertex buffer",
        ),
        (
            gpu.render(&[Target::cleared(&sampled, ClearValue::Color([0.0; 4]))], None, &[])
                .unwrap_err(),
            "a color target of Rgba8Unorm, level 0 of layer 0, is invalid: expected a texture made to be a target",
        ),
        (
            gpu.render(&[Target::cleared(&color, ClearValue::Uint([0; 4]))], None, &[])
                .unwrap_err(),
            "a color target of Rgba8Unorm, level 0 of layer 0, is invalid: expected a clear value for Rgba8Unorm, \
             not Uint([0, 0, 0, 0])",
        ),
    ];
    for (error, message) in refusals {
        assert!(matches!(error, GpuError::Invalid(_)), "{error:?}");
        assert_eq!(error.to_string(), message);
    }
}

#[test]
fn a_bind_group_binds_for_every_program_laid_out_alike() {
    let Some(gpu) = gpu("a_bind_group_binds_for_every_program_laid_out_alike") else {
        return;
    };
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    let near = gpu.render_pipeline(&flat(0.25), &state).unwrap();
    let far = gpu.render_pipeline(&flat(0.75), &state).unwrap();
    let (_uniforms, green) = colored(&gpu, &near, [0.0, 1.0, 0.0, 1.0]);
    let draw = Draw {
        groups: [None, Some(&green), None],
        ..Draw::new(&far, 3)
    };
    let pixels = draw_one(&gpu, TextureFormat::Rgba8Unorm, ClearValue::Color([0.0; 4]), draw);
    assert_eq!(
        texel(&pixels, SIZE / 2, SIZE / 2),
        GREEN,
        "another program's material group binds"
    );

    // A second member makes the material's block larger, and its layout another.
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    site.uniform(Frequency::Material, "color", jess::shader::DataType::Value(Type::Vec4f))
        .unwrap();
    site.uniform(Frequency::Material, "fade", jess::shader::DataType::Value(Type::Vec4f))
        .unwrap();
    let position = covering(&mut site, 0.5);
    site.set_position(position).unwrap();
    let color = site.float(1.0).unwrap();
    let color = site.construct(Type::Vec4f, &[color]).unwrap();
    site.output(color).unwrap();
    let wider = gpu.render_pipeline(&builder.finish().unwrap(), &state).unwrap();
    let color = target(&gpu, TextureFormat::Rgba8Unorm);
    let refused = gpu.render(
        &[Target::cleared(&color, ClearValue::Color([0.0; 4]))],
        None,
        &[Draw {
            groups: [None, Some(&green), None],
            ..Draw::new(&wider, 3)
        }],
    );
    assert_eq!(
        refused.unwrap_err().to_string(),
        "draw 0 is invalid: its material bind group was made for another frequency or layout"
    );
}

#[test]
fn indirect_draws_take_their_counts_from_a_buffer() {
    let Some(gpu) = gpu("indirect_draws_take_their_counts_from_a_buffer") else {
        return;
    };
    let state = PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        ..PipelineState::default()
    };
    let pipeline = gpu.render_pipeline(&flat(0.5), &state).unwrap();
    let (_red, red) = colored(&gpu, &pipeline, [1.0, 0.0, 0.0, 1.0]);
    let arguments = |usage| {
        let buffer = gpu.buffer(16, usage).unwrap();
        let counts: Vec<u8> = [3u32, 1, 0, 0].iter().flat_map(|count| count.to_le_bytes()).collect();
        gpu.write_buffer(&buffer, 0, &counts).unwrap();
        buffer
    };
    let indirect = arguments(BufferUsage::INDIRECT);
    let draw = |buffer, offset| Draw {
        groups: [None, Some(&red), None],
        indirect: Some((buffer, offset)),
        ..Draw::new(&pipeline, 0)
    };
    let pixels = draw_one(
        &gpu,
        TextureFormat::Rgba8Unorm,
        ClearValue::Color([0.0; 4]),
        draw(&indirect, 0),
    );
    assert_eq!(
        texel(&pixels, SIZE / 2, SIZE / 2),
        RED,
        "three vertices of one instance, from the buffer"
    );

    let plain = arguments(BufferUsage::STORAGE);
    let color = target(&gpu, TextureFormat::Rgba8Unorm);
    let clear = Target::cleared(&color, ClearValue::Color([0.0; 4]));
    for (draw, message) in [
        (draw(&plain, 0), "draw 0 is invalid: its indirect buffer isn't one"),
        (
            draw(&indirect, 4),
            "draw 0 is invalid: its indirect arguments take 16 bytes from 4, a multiple of 4, in its 16-byte buffer",
        ),
    ] {
        assert_eq!(gpu.render(&[clear], None, &[draw]).unwrap_err().to_string(), message);
    }
}
