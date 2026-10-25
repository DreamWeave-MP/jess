//! WGSL lowering: goldens for small programs, every one checked by naga, plus bindings and layout.

use super::common;

use common::{assert_golden, assert_valid_wgsl};
use jess::shader::layout::{MemberLayout, ResourceBinding};
use jess::shader::wgsl;
use jess::shader::{
    Attribute, BinaryOperator, Expression, ExpressionId, Frequency, Function, Literal, Program, Rate, Resource,
    ResourceKind, Swizzle, Type,
};

fn float(program: &mut Program, value: f32) -> ExpressionId {
    program.push(Expression::Literal(Literal::Float(value)))
}

/// Reads `frequency`'s uniform member `name`, declaring it as `ty` the first time.
fn read(program: &mut Program, frequency: Frequency, name: &str, ty: Type) -> ExpressionId {
    let place = common::uniform(program, frequency, name, ty);
    program.push(Expression::Load(place))
}

/// `projection * view * model * vec4(position, 1)`.
fn clip_position(program: &mut Program, position: ExpressionId) -> ExpressionId {
    let one = float(program, 1.0);
    let local = program.push(Expression::construct(Type::Vec4f, &[position, one]).unwrap());
    let model = read(program, Frequency::Draw, "model", Type::Mat4f);
    let view = read(program, Frequency::Pass, "view", Type::Mat4f);
    let projection = read(program, Frequency::Pass, "projection", Type::Mat4f);
    let world = program.push(Expression::Binary(BinaryOperator::Multiply, model, local));
    let eye = program.push(Expression::Binary(BinaryOperator::Multiply, view, world));
    program.push(Expression::Binary(BinaryOperator::Multiply, projection, eye))
}

fn solid_color() -> Program {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec3f));
    let color = common::uniform(&mut program, Frequency::Material, "color", Type::Vec4f);
    let position = program.push(Expression::Attribute(position));
    let clip = clip_position(&mut program, position);
    program.set_position(clip);
    let color = program.push(Expression::Load(color));
    program.push_fragment_output(color);
    program
}

/// A textured, lit mesh with the common transform, lighting, and texture inputs.
fn textured_lit() -> Program {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec3f));
    let normal = program.attribute(Attribute::new(Rate::Vertex, "normal", Type::Vec3f));
    let uv = program.attribute(Attribute::new(Rate::Vertex, "uv", Type::Vec2f));
    let normal_matrix = common::uniform(&mut program, Frequency::Material, "normalMatrix", Type::Mat3f);
    let light_direction = common::uniform(&mut program, Frequency::Material, "lightDirection", Type::Vec3f);
    let ambient = common::uniform(&mut program, Frequency::Material, "ambient", Type::Float);
    let diffuse_texture = program.resource(Resource::new(Frequency::Material, "diffuse", ResourceKind::Texture2D));
    let diffuse_sampler = program.resource(Resource::new(Frequency::Material, "diffuse", ResourceKind::Sampler));
    let world_normal = program.varying("normal", Type::Vec3f);
    let texture_coordinates = program.varying("uv", Type::Vec2f);

    let position = program.push(Expression::Attribute(position));
    let clip = clip_position(&mut program, position);
    program.set_position(clip);
    let normal = program.push(Expression::Attribute(normal));
    let normal_matrix = program.push(Expression::Load(normal_matrix));
    let transformed = program.push(Expression::Binary(BinaryOperator::Multiply, normal_matrix, normal));
    program.write_varying(world_normal, transformed);
    let uv = program.push(Expression::Attribute(uv));
    program.write_varying(texture_coordinates, uv);

    let (texture, sampler) = (diffuse_texture, diffuse_sampler);
    let uv = program.push(Expression::Varying(texture_coordinates));
    let diffuse = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates: uv,
        layer: None,
    });
    let normal = program.push(Expression::Varying(world_normal));
    let unit_normal = program.push(Expression::call(Function::Normalize, &[normal]).unwrap());
    let light_direction = program.push(Expression::Load(light_direction));
    let facing = program.push(Expression::call(Function::Dot, &[unit_normal, light_direction]).unwrap());
    let zero = float(&mut program, 0.0);
    let lambert = program.push(Expression::call(Function::Max, &[facing, zero]).unwrap());
    let ambient = program.push(Expression::Load(ambient));
    let light = program.push(Expression::Binary(BinaryOperator::Add, lambert, ambient));
    let rgb = program.push(Expression::Swizzle {
        vector: diffuse,
        swizzle: Swizzle::parse("xyz").unwrap(),
    });
    let lit = program.push(Expression::Binary(BinaryOperator::Multiply, rgb, light));
    let one = float(&mut program, 1.0);
    let clamped = program.push(Expression::call(Function::Clamp, &[lit, zero, one]).unwrap());
    let alpha = program.push(Expression::Swizzle {
        vector: diffuse,
        swizzle: Swizzle::parse("w").unwrap(),
    });
    let color = program.push(Expression::construct(Type::Vec4f, &[clamped, alpha]).unwrap());
    program.push_fragment_output(color);
    program
}

/// Integer varyings, several outputs, and the rest of the operations.
fn assorted() -> Program {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec4f));
    let index = program.attribute(Attribute::new(Rate::Vertex, "index", Type::UInt));
    let tint = common::uniform(&mut program, Frequency::Material, "tint", Type::Vec3f);
    let offset = common::uniform(&mut program, Frequency::Material, "offset", Type::Int);
    let flat_index = program.varying("index", Type::UInt);
    let signed = program.varying("signed", Type::Int);
    program.varying("unused", Type::Vec2f);

    let position = program.push(Expression::Attribute(position));
    program.set_position(position);
    let index = program.push(Expression::Attribute(index));
    program.write_varying(flat_index, index);
    let offset = program.push(Expression::Load(offset));
    let minimum = program.push(Expression::Literal(Literal::Int(i32::MIN)));
    let shifted = program.push(Expression::Binary(BinaryOperator::Subtract, offset, minimum));
    program.write_varying(signed, shifted);

    let tint = program.push(Expression::Load(tint));
    let up = program.push(Expression::Literal(Literal::Vec3f([0.0, 0.0, 1.0])));
    let side = program.push(Expression::call(Function::Cross, &[tint, up]).unwrap());
    let time = read(&mut program, Frequency::Pass, "time", Type::Float);
    let blended = program.push(Expression::call(Function::Mix, &[tint, side, time]).unwrap());
    let magnitude = program.push(Expression::call(Function::Length, &[blended]).unwrap());
    let viewport = read(&mut program, Frequency::Pass, "viewportSize", Type::Vec2f);
    let smallest = program.push(Expression::call(Function::Min, &[viewport, magnitude]).unwrap());
    let one = float(&mut program, 1.0);
    let first = program.push(Expression::construct(Type::Vec4f, &[blended, one]).unwrap());
    let half = float(&mut program, 0.5);
    let second = program.push(Expression::construct(Type::Vec4f, &[smallest, half, half]).unwrap());
    let quotient = program.push(Expression::Binary(BinaryOperator::Divide, second, magnitude));
    program.push_fragment_output(first);
    program.push_fragment_output(quotient);
    program
}

fn lowered(program: &Program) -> wgsl::WgslModule {
    let module = wgsl::lower(program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    module
}

#[test]
fn solid_color_matches_golden() {
    let program = solid_color();
    let module = lowered(&program);
    assert_golden("solid_color", &module.source);
    let size = |frequency: Frequency| module.layout.group(frequency).uniform.as_ref().map(|block| block.size);
    assert_eq!(Frequency::ALL.map(size), [Some(128), Some(16), Some(64)]);
    let pass: Vec<&str> = program
        .uniforms_of(Frequency::Pass)
        .map(|(_, uniform)| uniform.name.as_str())
        .collect();
    assert_eq!(pass, ["view", "projection"]);
    assert!(module.layout.groups.iter().all(|group| group.resources.is_empty()));
    assert_eq!(module.layout.vertex_buffers[0].attributes.len(), 1);
    assert_eq!(module.fragment_outputs, 1);
}

#[test]
fn textured_lit_matches_golden() {
    let program = textured_lit();
    let module = lowered(&program);
    assert_golden("textured_lit", &module.source);
    let resources: Vec<(u32, u32, ResourceKind)> = module
        .layout
        .group(Frequency::Material)
        .resources
        .iter()
        .map(
            |ResourceBinding {
                 resource,
                 group,
                 binding,
             }| { (*group, *binding, program.resources()[resource.index()].kind) },
        )
        .collect();
    assert_eq!(
        resources,
        [(1, 1, ResourceKind::Texture2D), (1, 2, ResourceKind::Sampler)],
        "the material block is binding 0"
    );
    let locations: Vec<(u32, Type)> = module.layout.vertex_buffers[0]
        .attributes
        .iter()
        .map(|attribute| (attribute.location, attribute.ty))
        .collect();
    assert_eq!(locations, [(0, Type::Vec3f), (1, Type::Vec3f), (2, Type::Vec2f)]);
}

#[test]
fn assorted_matches_golden() {
    let module = lowered(&assorted());
    assert_golden("assorted", &module.source);
    assert!(module.source.contains("@location(0) @interpolate(flat) v_index: u32"));
    assert!(!module.source.contains("v_unused"), "an unwritten varying is not an output");
    assert_eq!(module.fragment_outputs, 2);
}

#[test]
fn folded_program_matches_golden() {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec2f));
    let color = common::uniform(&mut program, Frequency::Material, "color", Type::Vec4f);
    let position = program.push(Expression::Attribute(position));
    let zero = float(&mut program, 0.0);
    let one = float(&mut program, 1.0);
    let clip = program.push(Expression::construct(Type::Vec4f, &[position, zero, one]).unwrap());
    let scaled = program.push(Expression::Binary(BinaryOperator::Multiply, clip, one));
    program.set_position(scaled);
    let half = float(&mut program, 0.5);
    let quarter = float(&mut program, 0.25);
    let sum = program.push(Expression::Binary(BinaryOperator::Add, half, quarter));
    let gray = program.push(Expression::construct(Type::Vec4f, &[sum]).unwrap());
    let color = program.push(Expression::Load(color));
    let tinted = program.push(Expression::Binary(BinaryOperator::Multiply, color, gray));
    let shifted = program.push(Expression::Binary(BinaryOperator::Add, tinted, zero));
    program.push_fragment_output(shifted);

    assert_eq!(program.fold_constants(), Ok(4));
    let module = lowered(&program);
    assert_golden("folded", &module.source);
}

#[test]
fn lowering_is_deterministic() {
    let first = lowered(&textured_lit());
    let second = lowered(&textured_lit());
    assert_eq!(first, second);
}

#[test]
fn only_reachable_expressions_are_lowered() {
    let mut program = solid_color();
    let before = lowered(&program).source;
    let dead = float(&mut program, 7.0);
    let doubled = program.push(Expression::Binary(BinaryOperator::Add, dead, dead));
    program.push(Expression::call(Function::Normalize, &[doubled]).unwrap());
    assert_eq!(lowered(&program).source, before);
}

#[test]
fn uniform_members_lie_largest_alignment_first() {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec4f));
    let names = [
        ("scale", Type::Float),
        ("tint", Type::Vec3f),
        ("fade", Type::Float),
        ("rotation", Type::Mat3f),
        ("offset", Type::Vec2f),
    ];
    let mut reads = Vec::new();
    for (name, ty) in names {
        reads.push(read(&mut program, Frequency::Material, name, ty));
    }
    let position = program.push(Expression::Attribute(position));
    program.set_position(position);
    let rotated = program.push(Expression::Binary(BinaryOperator::Multiply, reads[3], reads[1]));
    let scaled = program.push(Expression::Binary(BinaryOperator::Multiply, rotated, reads[0]));
    let faded = program.push(Expression::construct(Type::Vec4f, &[scaled, reads[2]]).unwrap());
    let shifted = program.push(Expression::construct(Type::Vec4f, &[reads[4], reads[4]]).unwrap());
    let color = program.push(Expression::Binary(BinaryOperator::Add, faded, shifted));
    program.push_fragment_output(color);

    let module = lowered(&program);
    let block = module.layout.group(Frequency::Material).uniform.as_ref().unwrap();
    let layout: Vec<(u32, u32)> = block
        .members
        .iter()
        .map(|MemberLayout { offset, size, .. }| (*offset, *size))
        .collect();
    // By declaration: the Vec3f and Mat3f lead, the Vec2f follows, the Floats fill the end.
    assert_eq!(layout, [(72, 4), (0, 12), (76, 4), (16, 48), (64, 8)]);
    assert_eq!(block.size, 80);
}

#[test]
fn instance_attributes_follow_the_vertex_attributes_and_matrices_arrive_by_column() {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec3f));
    let transform = program.attribute(Attribute::new(Rate::Instance, "transform", Type::Mat4f));
    let tint = program.attribute(Attribute::new(Rate::Instance, "tint", Type::Vec4f));
    let position = program.push(Expression::Attribute(position));
    let one = float(&mut program, 1.0);
    let local = program.push(Expression::construct(Type::Vec4f, &[position, one]).unwrap());
    let transform = program.push(Expression::Attribute(transform));
    let clip = program.push(Expression::Binary(BinaryOperator::Multiply, transform, local));
    program.set_position(clip);
    let passed = program.varying("tint", Type::Vec4f);
    let tint = program.push(Expression::Attribute(tint));
    program.write_varying(passed, tint);
    let read = program.push(Expression::Varying(passed));
    program.push_fragment_output(read);

    let module = lowered(&program);
    for line in [
        "@location(0) a_position: vec3<f32>",
        "@location(1) m1_transform: vec4<f32>",
        "@location(4) m4_transform: vec4<f32>",
        "@location(5) i_tint: vec4<f32>",
        "mat4x4<f32>(input.m1_transform, input.m2_transform, input.m3_transform, input.m4_transform)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
    let [vertices, instances] = &module.layout.vertex_buffers;
    assert_eq!((vertices.stride, instances.stride), (12, 80));
}

#[test]
fn attributes_take_at_most_sixteen_locations() {
    let mut program = Program::new();
    for name in ["a", "b", "c", "d", "e"] {
        program.attribute(Attribute::new(Rate::Instance, name, Type::Mat4f));
    }
    let position = float(&mut program, 0.0);
    let clip = program.push(Expression::construct(Type::Vec4f, &[position]).unwrap());
    program.set_position(clip);

    let errors = wgsl::lower(&program).unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(messages, ["the attributes take 20 locations: at most 16"]);
}
