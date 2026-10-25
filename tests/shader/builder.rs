//! The acceptance corpus a symbolic frontend must express, built through `ProgramBuilder`: each
//! program lowers to WGSL naga accepts, two of them to the same bytes as the hand-built IR in
//! `wgsl.rs`, and each mistake is reported at the site that made it.

use super::common;

use std::sync::Arc;

use common::{assert_golden, assert_valid_wgsl, read};
use jess::shader::wgsl::{self, WgslModule};
use jess::shader::{
    DataType, Frequency, Literal, Program, ProgramBuilder, Rate, ResourceKind, Site, SourceOrigin, Swizzle, Type,
    ValidationError, ValidationErrorKind, Value,
};

fn origin(line: u32, column: u32) -> SourceOrigin {
    SourceOrigin {
        owner: Arc::from("@test/viewer"),
        file: Arc::from("shaders/lit.jsl"),
        line,
        column,
    }
}

/// `projection * (view * (model * vec4(position, 1)))`, as `wgsl.rs` builds it by hand.
fn clip(site: &mut Site<'_>, position: Value) -> Result<Value, ValidationError> {
    let one = site.float(1.0)?;
    let local = site.construct(Type::Vec4f, &[position, one])?;
    let model = read(site, Frequency::Draw, "model", Type::Mat4f);
    let view = read(site, Frequency::Pass, "view", Type::Mat4f);
    let projection = read(site, Frequency::Pass, "projection", Type::Mat4f);
    let world = site.multiply(model, local)?;
    let eye = site.multiply(view, world)?;
    site.multiply(projection, eye)
}

fn lowered(builder: ProgramBuilder) -> (Program, WgslModule) {
    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    (program, module)
}

/// 12.1: a vertex position and a material color.
fn solid_color() -> Result<ProgramBuilder, ValidationError> {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec3f)?;
    let position = site.read_attribute(position);
    let color = read(&mut site, Frequency::Material, "color", Type::Vec4f);
    let clip = clip(&mut site, position)?;
    site.set_position(clip)?;
    site.output(color)?;
    Ok(builder)
}

/// 12.3, 12.4, and 12.5 together: a sampled texture, lit by a directional light, with the
/// swizzles and the vector construction that a real material needs.
fn textured_lit() -> Result<ProgramBuilder, ValidationError> {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec3f)?;
    let position = site.read_attribute(position);
    let normal = site.attribute(Rate::Vertex, "normal", Type::Vec3f)?;
    let normal = site.read_attribute(normal);
    let uv = site.attribute(Rate::Vertex, "uv", Type::Vec2f)?;
    let uv = site.read_attribute(uv);
    let normal_matrix = read(&mut site, Frequency::Material, "normalMatrix", Type::Mat3f);
    let light_direction = read(&mut site, Frequency::Material, "lightDirection", Type::Vec3f);
    let ambient = read(&mut site, Frequency::Material, "ambient", Type::Float);
    let texture = site.resource(Frequency::Material, "diffuse", ResourceKind::Texture2D)?;
    let sampler = site.resource(Frequency::Material, "diffuse", ResourceKind::Sampler)?;

    let clip = clip(&mut site, position)?;
    site.set_position(clip)?;
    let transformed = site.multiply(normal_matrix, normal)?;
    let normal = site.pass("normal", transformed)?;
    let uv = site.pass("uv", uv)?;

    let diffuse = site.sample(texture, sampler, uv)?;
    let unit_normal = site.normalize(normal)?;
    let facing = site.dot(unit_normal, light_direction)?;
    let zero = site.float(0.0)?;
    let lambert = site.max(facing, zero)?;
    let light = site.add(lambert, ambient)?;
    let rgb = site.swizzle(diffuse, Swizzle::parse("xyz").unwrap())?;
    let lit = site.multiply(rgb, light)?;
    let one = site.float(1.0)?;
    let clamped = site.clamp(lit, zero, one)?;
    let alpha = site.swizzle(diffuse, Swizzle::parse("w").unwrap())?;
    let color = site.construct(Type::Vec4f, &[clamped, alpha])?;
    site.output(color)?;
    Ok(builder)
}

#[test]
fn solid_color_lowers_like_the_hand_built_program() {
    let (program, module) = lowered(solid_color().unwrap());
    assert_golden("solid_color", &module.source);
    assert_eq!(program.varyings(), []);
}

#[test]
fn textured_lit_lowers_like_the_hand_built_program() {
    let (program, module) = lowered(textured_lit().unwrap());
    assert_golden("textured_lit", &module.source);
    let varyings: Vec<(&str, Type)> = program
        .varyings()
        .iter()
        .map(|declaration| (declaration.name.as_str(), declaration.ty))
        .collect();
    assert_eq!(varyings, [("normal", Type::Vec3f), ("uv", Type::Vec2f)]);
}

/// 12.2: the handoff's final success test, `projection * view * model * vec4(position, 1)` as
/// Luau evaluates it (matrix products first), a color passed to the fragment stage, and a tint.
/// Every site has an origin, and the program keeps all of them.
#[test]
fn vertex_transform_and_color_keep_their_origins() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.at(origin(4, 5));
    let tint = read(&mut site, Frequency::Material, "tint", Type::Vec4f);
    let position = site.attribute(Rate::Vertex, "position", Type::Vec3f).unwrap();
    let position = site.read_attribute(position);
    let color = site.attribute(Rate::Vertex, "color", Type::Vec4f).unwrap();
    let color = site.read_attribute(color);

    let mut site = builder.at(origin(9, 17));
    let projection = read(&mut site, Frequency::Pass, "projection", Type::Mat4f);
    let view = read(&mut site, Frequency::Pass, "view", Type::Mat4f);
    let model = read(&mut site, Frequency::Draw, "model", Type::Mat4f);
    let one = site.float(1.0).unwrap();
    let projection_view = site.multiply(projection, view).unwrap();
    let transform = site.multiply(projection_view, model).unwrap();
    assert_eq!(transform.ty(), Type::Mat4f);
    let local = site.construct(Type::Vec4f, &[position, one]).unwrap();
    let clip = site.multiply(transform, local).unwrap();
    assert_eq!(clip.ty(), Type::Vec4f);
    site.set_position(clip).unwrap();

    let mut site = builder.at(origin(14, 7));
    let color = site.pass("color", color).unwrap();
    assert_eq!(color.ty(), Type::Vec4f);

    let mut site = builder.at(origin(18, 12));
    let tinted = site.multiply(color, tint).unwrap();
    site.output(tinted).unwrap();

    let program = builder.program();
    assert_eq!(program.uniforms()[0].origin, Some(origin(4, 5)));
    assert_eq!(program.attributes()[1].origin, Some(origin(4, 5)));
    assert_eq!(program.varyings()[0].name, "color");
    assert_eq!(program.varyings()[0].origin, Some(origin(14, 7)));
    assert_eq!(program.origin(clip.expression()), Some(&origin(9, 17)));
    assert_eq!(program.origin(color.expression()), Some(&origin(14, 7)));
    assert_eq!(program.origin(tinted.expression()), Some(&origin(18, 12)));

    let (_, module) = lowered(builder);
    assert!(module.source.contains("let e0 = u_pass.f_projection * u_pass.f_view;"));
    assert!(module.source.contains("output.v_color = input.a_color;"));
    assert!(module.source.contains("let e0 = input.v_color * u_material.f_tint;"));
}

/// 12.5 on its own: every swizzle shape, and construction from a vector and floats, from two
/// vectors, and from one float repeated. Each value knows its type as soon as it exists.
#[test]
fn swizzles_and_constructions_are_typed_as_they_are_built() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec2f).unwrap();
    let position = site.read_attribute(position);
    let tint = read(&mut site, Frequency::Material, "tint", Type::Vec3f);
    let zero = site.float(0.0).unwrap();
    let one = site.float(1.0).unwrap();
    let clip = site.construct(Type::Vec4f, &[position, zero, one]).unwrap();
    assert_eq!(clip.ty(), Type::Vec4f);
    site.set_position(clip).unwrap();

    let reversed = site.swizzle(tint, Swizzle::parse("zyx").unwrap()).unwrap();
    let red = site.swizzle(tint, Swizzle::parse("x").unwrap()).unwrap();
    let pair = site.swizzle(reversed, Swizzle::parse("xy").unwrap()).unwrap();
    assert_eq!((reversed.ty(), red.ty(), pair.ty()), (Type::Vec3f, Type::Float, Type::Vec2f));
    let gray = site.construct(Type::Vec4f, &[red]).unwrap();
    let doubled = site.construct(Type::Vec4f, &[pair, pair]).unwrap();
    assert_eq!((gray.ty(), doubled.ty()), (Type::Vec4f, Type::Vec4f));
    let color = site.add(gray, doubled).unwrap();
    site.output(color).unwrap();

    let (_, module) = lowered(builder);
    assert!(module.source.contains("let e0 = u_material.f_tint.zyx;"));
    assert!(module.source.contains("let e1 = u_material.f_tint.x;"));
    assert!(module.source.contains("let e3 = vec4<f32>(e1);"));
    assert!(module.source.contains("let e4 = vec4<f32>(e2, e2);"));
}

/// 12.6: the mistake fails at the call that made it, with that call's origin, and pushes nothing.
#[test]
fn dot_of_different_sizes_fails_at_its_site() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let normal = site.literal(Literal::Vec3f([0.0, 0.0, 1.0])).unwrap();
    let light = site.literal(Literal::Vec4f([0.0, 1.0, 0.0, 0.0])).unwrap();
    let before = builder.program().expressions().len();

    let error = builder.at(origin(12, 5)).dot(normal, light).unwrap_err();
    assert_eq!(
        error.to_string(),
        "shaders/lit.jsl:12:5 (@test/viewer): dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type"
    );
    assert!(matches!(
        error.kind,
        ValidationErrorKind::InvalidOperation {
            operation: "dot",
            ref operands,
            ..
        } if operands == &[Type::Vec3f, Type::Vec4f]
    ));
    assert_eq!(
        builder.program().expressions().len(),
        before,
        "a rejected operation is not pushed"
    );

    // The builder is still usable: fix the mistake and finish.
    let mut site = builder.at(origin(12, 5));
    let light = site.swizzle(light, Swizzle::parse("xyz").unwrap()).unwrap();
    let facing = site.dot(normal, light).unwrap();
    assert_eq!(facing.ty(), Type::Float);
    let position = site.construct(Type::Vec4f, &[facing]).unwrap();
    site.set_position(position).unwrap();
    assert!(builder.finish().is_ok());
}

#[test]
fn every_operation_is_typed_at_its_site() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.at(origin(30, 1));
    let scalar = site.float(1.0).unwrap();
    let integer = site.int(1);
    let pair = site.literal(Literal::Vec2f([1.0, 2.0])).unwrap();
    let triple = site.literal(Literal::Vec3f([1.0, 2.0, 3.0])).unwrap();
    let quad = site.literal(Literal::Vec4f([1.0, 2.0, 3.0, 4.0])).unwrap();
    let model = read(&mut site, Frequency::Draw, "model", Type::Mat4f);
    let texture = site
        .resource(Frequency::Material, "diffuse", ResourceKind::Texture2D)
        .unwrap();
    let before = builder.program().expressions().len();

    let mut site = builder.at(origin(30, 1));
    let messages = [
        site.min(integer, scalar).unwrap_err().to_string(),
        site.multiply(model, triple).unwrap_err().to_string(),
        site.divide(model, model).unwrap_err().to_string(),
        site.cross(quad, quad).unwrap_err().to_string(),
        site.mix(pair, triple, scalar).unwrap_err().to_string(),
        site.clamp(triple, pair, scalar).unwrap_err().to_string(),
        site.length(integer).unwrap_err().to_string(),
        site.normalize(scalar).unwrap_err().to_string(),
        site.sample(texture, texture, pair).unwrap_err().to_string(),
        site.construct(Type::Vec4f, &[triple, pair]).unwrap_err().to_string(),
        site.construct(Type::Vec4f, &[scalar, scalar, scalar, scalar, scalar])
            .unwrap_err()
            .to_string(),
        site.construct(Type::Mat3f, &[triple, triple, triple])
            .unwrap_err()
            .to_string(),
        site.swizzle(pair, Swizzle::parse("xyz").unwrap()).unwrap_err().to_string(),
        site.swizzle(scalar, Swizzle::parse("x").unwrap()).unwrap_err().to_string(),
    ];
    let expected = [
        "min(Int, Float) is invalid: expected two values of the same numeric type, or a vector and a scalar of its kind",
        "Mat4f * Vec3f is invalid: expected two values of the same numeric type, a vector and a scalar of its kind, a matrix \
         and a Float, a matrix and a vector of its size, or two matrices of the same size",
        "Mat4f / Mat4f is invalid: expected two values of the same numeric type, or a vector and a scalar of its kind",
        "cross(Vec4f, Vec4f) is invalid: expected two Vec3f",
        "mix(Vec2f, Vec3f, Float) is invalid: expected two Floats or float vectors of the same type, and an amount of \
         that type or Float",
        "clamp(Vec3f, Vec2f, Float) is invalid: expected a numeric value and two bounds of its type, or a vector \
         and two scalars of its kind",
        "length(Int) is invalid: expected a Float or float vector",
        "normalize(Float) is invalid: expected a float vector",
        "sample(Texture2D, Texture2D, Vec2f) is invalid: expected sample(Texture2D, Sampler, Vec2f)",
        "Vec4f(Vec3f, Vec2f) is invalid: expected floats and float vectors adding up to 4 components, or one Float",
        "Vec4f(Float, Float, Float, Float, Float) is invalid: expected floats and float vectors adding up to 4 \
         components, or one Float",
        "Mat3f(Vec3f, Vec3f, Vec3f) is invalid: expected a vector target",
        "Vec2f.xyz is invalid: expected components a Vec2f has",
        "Float.x is invalid: expected components a Float has",
    ];
    let prefix = "shaders/lit.jsl:30:1 (@test/viewer): ";
    for (message, expected) in messages.iter().zip(expected) {
        assert_eq!(message, &format!("{prefix}{expected}"));
    }
    assert_eq!(builder.program().expressions().len(), before);
}

/// A frontend whose source only records lines (Luau) gives a column of 0, which prints as
/// `file:line`; nothing invents a column.
#[test]
fn an_origin_without_a_column_prints_the_line_alone() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.at(origin(12, 0));
    let normal = site.literal(Literal::Vec3f([0.0, 0.0, 1.0])).unwrap();
    let light = site.literal(Literal::Vec4f([0.0, 1.0, 0.0, 0.0])).unwrap();
    assert_eq!(
        site.dot(normal, light).unwrap_err().to_string(),
        "shaders/lit.jsl:12 (@test/viewer): dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type"
    );
}

#[test]
fn literals_wgsl_cannot_write_fail_at_their_site() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.at(origin(2, 9));
    assert_eq!(
        site.float(f32::NAN).unwrap_err().to_string(),
        "shaders/lit.jsl:2:9 (@test/viewer): expression 0 is a NaN or infinite literal, which WGSL cannot write"
    );
    let largest = site.int(i32::MAX);
    let one = site.int(1);
    let seven = site.uint(7);
    let zero = site.uint(0);
    let huge = site.float(3.0e38).unwrap();
    let overflow = site.add(largest, one).unwrap_err();
    let division = site.divide(seven, zero).unwrap_err();
    let infinite = site.multiply(huge, huge).unwrap_err();
    for error in [overflow, division, infinite] {
        assert!(
            matches!(error.kind, ValidationErrorKind::ConstantOutOfRange { .. }),
            "{error}"
        );
        assert_eq!(error.origin, Some(origin(2, 9)));
    }
    assert_eq!(site.divide(largest, one).unwrap().ty(), Type::Int);
}

#[test]
fn declarations_are_checked_at_their_site() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.at(origin(1, 1));
    site.uniform(Frequency::Material, "tint", DataType::Value(Type::Vec4f))
        .unwrap();
    // A texture and the sampler declared with it share a name.
    site.resource(Frequency::Material, "diffuse", ResourceKind::Texture2D)
        .unwrap();
    site.resource(Frequency::Material, "diffuse", ResourceKind::Sampler).unwrap();
    // Another frequency's names are its own.
    site.uniform(Frequency::Pass, "tint", DataType::Value(Type::Vec4f)).unwrap();
    let model = read(&mut site, Frequency::Draw, "model", Type::Mat4f);
    let uv = site.literal(Literal::Vec2f([0.5, 0.5])).unwrap();
    site.pass("uv", uv).unwrap();
    let declared = (builder.program().uniforms().len(), builder.program().resources().len());

    let mut site = builder.at(origin(1, 1));
    let messages = [
        site.uniform(Frequency::Material, "tint", DataType::Value(Type::Vec3f))
            .unwrap_err()
            .to_string(),
        site.uniform(Frequency::Material, "visible", DataType::Value(Type::Bool))
            .unwrap_err()
            .to_string(),
        site.resource(Frequency::Material, "tint", ResourceKind::Texture2D)
            .unwrap_err()
            .to_string(),
        site.uniform(Frequency::Material, "diffuse", DataType::Value(Type::Float))
            .unwrap_err()
            .to_string(),
        site.attribute(Rate::Vertex, "1st", Type::Float).unwrap_err().to_string(),
        site.attribute(Rate::Vertex, "flag", Type::Bool).unwrap_err().to_string(),
        site.pass("transform", model).unwrap_err().to_string(),
        site.pass("uv", uv).unwrap_err().to_string(),
        site.pass("light-direction", uv).unwrap_err().to_string(),
    ];
    assert_eq!(
        messages,
        [
            "shaders/lit.jsl:1:1 (@test/viewer): material uniform \"tint\" is declared twice",
            "shaders/lit.jsl:1:1 (@test/viewer): material uniform \"visible\" cannot be Bool: expected Int, UInt, \
             Float, a vector, a matrix, a struct, or an array of one of those",
            "shaders/lit.jsl:1:1 (@test/viewer): material texture \"tint\" is declared twice",
            "shaders/lit.jsl:1:1 (@test/viewer): material uniform \"diffuse\" is declared twice",
            "shaders/lit.jsl:1:1 (@test/viewer): attribute name \"1st\" is invalid: expected ASCII letters, \
             digits, and underscores, not starting with a digit",
            "shaders/lit.jsl:1:1 (@test/viewer): attribute \"flag\" cannot be Bool: expected Int, UInt, Float, \
             a vector, or a matrix",
            "shaders/lit.jsl:1:1 (@test/viewer): varying \"transform\" cannot be Mat4f: expected Int, UInt, \
             Float, or a vector",
            "shaders/lit.jsl:1:1 (@test/viewer): varying \"uv\" is declared twice",
            "shaders/lit.jsl:1:1 (@test/viewer): varying name \"light-direction\" is invalid: expected ASCII \
             letters, digits, and underscores, not starting with a digit",
        ]
    );
    let program = builder.program();
    assert_eq!((program.uniforms().len(), program.resources().len()), declared);
    assert_eq!(program.attributes(), []);
    assert_eq!(program.varying_writes().len(), 1, "a rejected pass writes nothing");
}

#[test]
fn values_from_another_program_are_refused() {
    let mut other = ProgramBuilder::new();
    let foreign = other.site().float(1.0).unwrap();

    let mut builder = ProgramBuilder::new();
    let mut site = builder.at(origin(7, 3));
    let own = site.float(2.0).unwrap();
    assert_eq!(own.expression(), foreign.expression(), "ids alias across programs");
    assert_ne!(own, foreign);

    let expected = "shaders/lit.jsl:7:3 (@test/viewer): expression 0 belongs to another shader program";
    assert_eq!(site.add(own, foreign).unwrap_err().to_string(), expected);
    assert_eq!(site.add(foreign, own).unwrap_err().to_string(), expected);
    assert_eq!(site.normalize(foreign).unwrap_err().to_string(), expected);
    assert_eq!(
        site.construct(Type::Vec4f, &[own, foreign]).unwrap_err().to_string(),
        expected
    );
    assert_eq!(site.pass("value", foreign).unwrap_err().to_string(), expected);
    assert_eq!(site.set_position(foreign).unwrap_err().to_string(), expected);
    assert_eq!(site.output(foreign).unwrap_err().to_string(), expected);
    assert_eq!(builder.program().expressions().len(), 1);
    assert!(builder.program().position().is_none());
    assert_eq!(builder.program().fragment_outputs(), []);
}

/// What only the whole program can tell: `finish` reports it, at the origin of the expression
/// or declaration at fault.
#[test]
fn finish_reports_whole_program_mistakes_with_origins() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.at(origin(3, 1));
    let position = site.attribute(Rate::Vertex, "position", Type::Vec3f).unwrap();
    let position = site.read_attribute(position);
    site.set_position(position).unwrap();

    let mut site = builder.at(origin(9, 2));
    let color = site.attribute(Rate::Vertex, "color", Type::Vec4f).unwrap();
    let color = site.read_attribute(color);
    site.output(color).unwrap();

    let mut site = builder.at(origin(25, 4));
    let texture = site
        .resource(Frequency::Material, "diffuse", ResourceKind::Texture2D)
        .unwrap();
    let sampler = site.resource(Frequency::Material, "diffuse", ResourceKind::Sampler).unwrap();
    let uv = site.literal(Literal::Vec2f([0.5, 0.5])).unwrap();
    let texel = site.sample(texture, sampler, uv).unwrap();
    site.pass("texel", texel).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "shaders/lit.jsl:9:2 (@test/viewer): attribute \"color\" is read in the fragment stage: pass it \
             through a varying",
            "shaders/lit.jsl:25:4 (@test/viewer): sample is used in the vertex stage: textures are sampled in the \
             fragment stage",
            "shaders/lit.jsl:3:1 (@test/viewer): the vertex position is Vec3f: expected Vec4f",
        ]
    );
}
