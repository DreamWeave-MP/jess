//! The native proof: shader IR, to WGSL, to wgpu, to two instances of a triangle read back from an
//! offscreen texture, drawn from nothing but the layout the lowering reported.
//!
//! Skips, loudly, when the machine has no adapter at all (not even a software one).

use super::common;

use jess::shader::layout::VertexBufferLayout;
use jess::shader::wgsl::{self, FRAGMENT_ENTRY_POINT, PARAMETER_GROUP, VERTEX_ENTRY_POINT, WgslModule};
use jess::shader::{Attribute, Expression, Literal, Program, Rate, Type};

const SIZE: u32 = 64;
const CLEAR: [u8; 4] = [0, 0, 255, 255];
const TRIANGLE: [u8; 4] = [0, 255, 0, 255];

fn triangle_program() -> Program {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec2f));
    let transform = program.attribute(Attribute::new(Rate::Instance, "transform", Type::Mat4f));
    let color = program.parameter("color", Type::Vec4f);
    let position = program.push(Expression::Attribute(position));
    let depth = program.push(Expression::Literal(Literal::Float(0.0)));
    let one = program.push(Expression::Literal(Literal::Float(1.0)));
    let local = program.push(Expression::construct(Type::Vec4f, &[position, depth, one]).unwrap());
    let transform = program.push(Expression::Attribute(transform));
    let clip = program.push(Expression::Multiply(transform, local));
    program.set_position(clip);
    let color = program.push(Expression::Parameter(color));
    let unchanged = program.push(Expression::Multiply(color, one));
    program.push_fragment_output(unchanged);
    program
}

fn request_adapter(instance: &wgpu::Instance) -> Option<wgpu::Adapter> {
    for force_fallback_adapter in [false, true] {
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            force_fallback_adapter,
            compatible_surface: None,
            ..Default::default()
        };
        if let Ok(adapter) = pollster::block_on(instance.request_adapter(&options)) {
            return Some(adapter);
        }
    }
    None
}

#[test]
fn shader_ir_draws_instanced_triangles() {
    let mut program = triangle_program();
    assert_eq!(program.fold_constants(), Ok(1));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    common::assert_valid_wgsl(&module);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Some(adapter) = request_adapter(&instance) else {
        eprintln!("SKIPPED shader_ir_draws_instanced_triangles: no wgpu adapter on this machine, not even a software one");
        return;
    };
    let info = adapter.get_info();
    eprintln!("triangle proof on {} ({:?}, {:?})", info.name, info.backend, info.device_type);

    let pixels = render(&adapter, &module);
    let pixel = |x: u32, y: u32| {
        let start = ((y * SIZE + x) * 4) as usize;
        [pixels[start], pixels[start + 1], pixels[start + 2], pixels[start + 3]]
    };
    assert_eq!(pixel(SIZE / 4, SIZE / 2), TRIANGLE, "the first instance is on the left");
    assert_eq!(pixel(SIZE * 3 / 4, SIZE / 2), TRIANGLE, "the second instance is on the right");
    assert_eq!(pixel(SIZE / 2, SIZE / 2), CLEAR, "the instances are apart");
    assert_eq!(pixel(0, 0), CLEAR, "a corner is the clear color");
    assert_eq!(pixel(SIZE - 1, SIZE - 1), CLEAR, "a corner is the clear color");
}

/// Draws `module` with nothing but what the lowering reported: its attributes, its parameter
/// block, and its entry points.
#[allow(clippy::too_many_lines)] // one wgpu setup, top to bottom
fn render(adapter: &wgpu::Adapter, module: &WgslModule) -> Vec<u8> {
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("triangle"),
        source: wgpu::ShaderSource::Wgsl(module.source.as_str().into()),
    });

    let parameters = module.parameters.as_ref().unwrap();
    assert_eq!((parameters.group, parameters.binding), (PARAMETER_GROUP, 0));
    let parameters_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("parameters"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: parameters.binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let color_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parameters"),
        size: u64::from(parameters.size),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let color: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
    let bytes: Vec<u8> = color.iter().flat_map(|value| value.to_le_bytes()).collect();
    queue.write_buffer(&color_buffer, u64::from(parameters.members[0].offset), &bytes);
    let parameters_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("parameters"),
        layout: &parameters_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: parameters.binding,
            resource: color_buffer.as_entire_binding(),
        }],
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("triangle"),
        // The program reads no semantics, so group 0 is a gap.
        bind_group_layouts: &[None, Some(&parameters_layout)],
        immediate_size: 0,
    });

    let [vertices, instances] = &module.layout.vertex_buffers;
    assert_eq!((vertices.stride, instances.stride), (8, 64));
    let vertex_attributes = wgpu_attributes(vertices);
    let instance_attributes = wgpu_attributes(instances);
    // The triangle at half size, once left of the center and once right of it: column-major.
    let corners = [-0.5f32, -0.5, 0.5, -0.5, 0.0, 0.5];
    let mut transforms = Vec::new();
    for offset in [-0.5f32, 0.5] {
        transforms.extend([
            0.5, 0.0, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, offset, 0.0, 0.0, 1.0,
        ]);
    }
    let vertex_buffer = upload(&device, &queue, "vertices", &corners);
    let instance_buffer = upload(&device, &queue, "instances", &transforms);

    let format = wgpu::TextureFormat::Rgba8Unorm;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("triangle"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some(VERTEX_ENTRY_POINT),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: u64::from(vertices.stride),
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &vertex_attributes,
                }),
                Some(wgpu::VertexBufferLayout {
                    array_stride: u64::from(instances.stride),
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &instance_attributes,
                }),
            ],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(FRAGMENT_ENTRY_POINT),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    // 64 pixels × 4 bytes is exactly the 256-byte row alignment copies need.
    let row_bytes = SIZE * 4;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: u64::from(row_bytes * SIZE),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("triangle") });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("triangle"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(PARAMETER_GROUP, &parameters_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        pass.draw(0..3, 0..2);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(SIZE),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);

    readback.slice(..).map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let pixels = readback.slice(..).get_mapped_range().unwrap().to_vec();
    readback.unmap();
    pixels
}

/// One buffer's attributes as wgpu takes them: a value at its location, or a matrix as one
/// `Float32x4` column per location.
fn wgpu_attributes(buffer: &VertexBufferLayout) -> Vec<wgpu::VertexAttribute> {
    let mut attributes = Vec::new();
    for attribute in &buffer.attributes {
        let (format, size) = match attribute.ty {
            Type::Vec2f => (wgpu::VertexFormat::Float32x2, 8),
            Type::Mat4f => (wgpu::VertexFormat::Float32x4, 16),
            other => panic!("the proof draws no {other} attribute"),
        };
        for column in 0..attribute.locations {
            attributes.push(wgpu::VertexAttribute {
                format,
                offset: u64::from(attribute.offset + column * size),
                shader_location: attribute.location + column,
            });
        }
    }
    attributes
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, values: &[f32]) -> wgpu::Buffer {
    let bytes: Vec<u8> = values.iter().flat_map(|value| value.to_le_bytes()).collect();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, &bytes);
    buffer
}
