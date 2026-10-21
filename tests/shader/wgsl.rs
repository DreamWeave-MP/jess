//! WGSL lowering: goldens for small programs, every one checked by naga, plus bindings and layout.

use super::common;

use common::{assert_golden, assert_valid_wgsl};
use jess::shader::wgsl::{self, PARAMETER_GROUP, ResourceBinding, SEMANTIC_GROUP, UniformMember};
use jess::shader::{Expression, ExpressionId, Literal, Program, Semantic, Swizzle, Type};

fn float(program: &mut Program, value: f32) -> ExpressionId {
    program.push(Expression::Literal(Literal::Float(value)))
}

/// `projection * view * model * vec4(position, 1)`.
fn clip_position(program: &mut Program, position: ExpressionId) -> ExpressionId {
    let one = float(program, 1.0);
    let local = program.push(Expression::construct(Type::Vec4f, &[position, one]).unwrap());
    let model = program.push(Expression::Semantic(Semantic::Model));
    let view = program.push(Expression::Semantic(Semantic::View));
    let projection = program.push(Expression::Semantic(Semantic::Projection));
    let world = program.push(Expression::Multiply(model, local));
    let eye = program.push(Expression::Multiply(view, world));
    program.push(Expression::Multiply(projection, eye))
}

fn solid_color() -> Program {
    let mut program = Program::new();
    let position = program.vertex_input("position", Type::Vec3f);
    let color = program.parameter("color", Type::Vec4f);
    let position = program.push(Expression::VertexInput(position));
    let clip = clip_position(&mut program, position);
    program.set_position(clip);
    let color = program.push(Expression::Parameter(color));
    program.push_fragment_output(color);
    program
}

/// A textured, lit mesh with the common transform, lighting, and texture inputs.
fn textured_lit() -> Program {
    let mut program = Program::new();
    let position = program.vertex_input("position", Type::Vec3f);
    let normal = program.vertex_input("normal", Type::Vec3f);
    let uv = program.vertex_input("uv", Type::Vec2f);
    let normal_matrix = program.parameter("normalMatrix", Type::Mat3f);
    let light_direction = program.parameter("lightDirection", Type::Vec3f);
    let ambient = program.parameter("ambient", Type::Float);
    let diffuse_texture = program.parameter("diffuse", Type::Texture2D);
    let diffuse_sampler = program.parameter("diffuse", Type::Sampler);
    let world_normal = program.varying("normal", Type::Vec3f);
    let texture_coordinates = program.varying("uv", Type::Vec2f);

    let position = program.push(Expression::VertexInput(position));
    let clip = clip_position(&mut program, position);
    program.set_position(clip);
    let normal = program.push(Expression::VertexInput(normal));
    let normal_matrix = program.push(Expression::Parameter(normal_matrix));
    let transformed = program.push(Expression::Multiply(normal_matrix, normal));
    program.write_varying(world_normal, transformed);
    let uv = program.push(Expression::VertexInput(uv));
    program.write_varying(texture_coordinates, uv);

    let texture = program.push(Expression::Parameter(diffuse_texture));
    let sampler = program.push(Expression::Parameter(diffuse_sampler));
    let uv = program.push(Expression::Varying(texture_coordinates));
    let diffuse = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates: uv,
    });
    let normal = program.push(Expression::Varying(world_normal));
    let unit_normal = program.push(Expression::Normalize(normal));
    let light_direction = program.push(Expression::Parameter(light_direction));
    let facing = program.push(Expression::Dot(unit_normal, light_direction));
    let zero = float(&mut program, 0.0);
    let lambert = program.push(Expression::Max(facing, zero));
    let ambient = program.push(Expression::Parameter(ambient));
    let light = program.push(Expression::Add(lambert, ambient));
    let rgb = program.push(Expression::Swizzle {
        vector: diffuse,
        swizzle: Swizzle::parse("xyz").unwrap(),
    });
    let lit = program.push(Expression::Multiply(rgb, light));
    let one = float(&mut program, 1.0);
    let clamped = program.push(Expression::Clamp(lit, zero, one));
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
    let position = program.vertex_input("position", Type::Vec4f);
    let index = program.vertex_input("index", Type::UInt);
    let tint = program.parameter("tint", Type::Vec3f);
    let offset = program.parameter("offset", Type::Int);
    let flat_index = program.varying("index", Type::UInt);
    let signed = program.varying("signed", Type::Int);
    program.varying("unused", Type::Vec2f);

    let position = program.push(Expression::VertexInput(position));
    program.set_position(position);
    let index = program.push(Expression::VertexInput(index));
    program.write_varying(flat_index, index);
    let offset = program.push(Expression::Parameter(offset));
    let minimum = program.push(Expression::Literal(Literal::Int(i32::MIN)));
    let shifted = program.push(Expression::Subtract(offset, minimum));
    program.write_varying(signed, shifted);

    let tint = program.push(Expression::Parameter(tint));
    let up = program.push(Expression::Literal(Literal::Vec3f([0.0, 0.0, 1.0])));
    let side = program.push(Expression::Cross(tint, up));
    let time = program.push(Expression::Semantic(Semantic::Time));
    let blended = program.push(Expression::Mix(tint, side, time));
    let magnitude = program.push(Expression::Length(blended));
    let viewport = program.push(Expression::Semantic(Semantic::ViewportSize));
    let smallest = program.push(Expression::Min(viewport, magnitude));
    let one = float(&mut program, 1.0);
    let first = program.push(Expression::construct(Type::Vec4f, &[blended, one]).unwrap());
    let half = float(&mut program, 0.5);
    let second = program.push(Expression::construct(Type::Vec4f, &[smallest, half, half]).unwrap());
    let quotient = program.push(Expression::Divide(second, magnitude));
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
    let module = lowered(&solid_color());
    assert_golden("solid_color", &module.source);
    let semantics = module.semantics.unwrap();
    assert_eq!((semantics.group, semantics.binding, semantics.size), (SEMANTIC_GROUP, 0, 192));
    let order: Vec<Semantic> = semantics.members.iter().map(|member| member.source).collect();
    assert_eq!(order, [Semantic::Model, Semantic::View, Semantic::Projection]);
    let parameters = module.parameters.unwrap();
    assert_eq!(
        (parameters.group, parameters.binding, parameters.size),
        (PARAMETER_GROUP, 0, 16)
    );
    assert_eq!(module.resources, []);
    assert_eq!(module.vertex_attributes.len(), 1);
    assert_eq!(module.fragment_outputs, 1);
}

#[test]
fn textured_lit_matches_golden() {
    let program = textured_lit();
    let module = lowered(&program);
    assert_golden("textured_lit", &module.source);
    let resources: Vec<(u32, u32, Type)> = module
        .resources
        .iter()
        .map(|ResourceBinding { group, binding, ty, .. }| (*group, *binding, *ty))
        .collect();
    assert_eq!(
        resources,
        [(PARAMETER_GROUP, 1, Type::Texture2D), (PARAMETER_GROUP, 2, Type::Sampler)]
    );
    let locations: Vec<(u32, Type)> = module
        .vertex_attributes
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
    let position = program.vertex_input("position", Type::Vec2f);
    let color = program.parameter("color", Type::Vec4f);
    let position = program.push(Expression::VertexInput(position));
    let zero = float(&mut program, 0.0);
    let one = float(&mut program, 1.0);
    let clip = program.push(Expression::construct(Type::Vec4f, &[position, zero, one]).unwrap());
    let scaled = program.push(Expression::Multiply(clip, one));
    program.set_position(scaled);
    let half = float(&mut program, 0.5);
    let quarter = float(&mut program, 0.25);
    let sum = program.push(Expression::Add(half, quarter));
    let gray = program.push(Expression::construct(Type::Vec4f, &[sum]).unwrap());
    let color = program.push(Expression::Parameter(color));
    let tinted = program.push(Expression::Multiply(color, gray));
    let shifted = program.push(Expression::Add(tinted, zero));
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
    let doubled = program.push(Expression::Add(dead, dead));
    program.push(Expression::Normalize(doubled));
    assert_eq!(lowered(&program).source, before);
}

#[test]
fn uniform_layout_follows_wgsl_alignment() {
    let mut program = Program::new();
    let position = program.vertex_input("position", Type::Vec4f);
    let names = [
        ("scale", Type::Float),
        ("tint", Type::Vec3f),
        ("fade", Type::Float),
        ("rotation", Type::Mat3f),
        ("offset", Type::Vec2f),
    ];
    let mut reads = Vec::new();
    for (name, ty) in names {
        let parameter = program.parameter(name, ty);
        reads.push(program.push(Expression::Parameter(parameter)));
    }
    let position = program.push(Expression::VertexInput(position));
    program.set_position(position);
    let rotated = program.push(Expression::Multiply(reads[3], reads[1]));
    let scaled = program.push(Expression::Multiply(rotated, reads[0]));
    let faded = program.push(Expression::construct(Type::Vec4f, &[scaled, reads[2]]).unwrap());
    let shifted = program.push(Expression::construct(Type::Vec4f, &[reads[4], reads[4]]).unwrap());
    let color = program.push(Expression::Add(faded, shifted));
    program.push_fragment_output(color);

    let module = lowered(&program);
    let parameters = module.parameters.unwrap();
    let layout: Vec<(u32, u32)> = parameters
        .members
        .iter()
        .map(|UniformMember { offset, size, .. }| (*offset, *size))
        .collect();
    assert_eq!(layout, [(0, 4), (16, 12), (28, 4), (32, 48), (80, 8)]);
    assert_eq!(parameters.size, 96);
}
