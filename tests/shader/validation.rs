//! Validation reports mistakes in Jess terms before any backend sees the program.

use std::sync::Arc;

use jess::shader::{
    DeclarationKind, Expression, ExpressionId, Literal, Program, Semantic, SourceOrigin, Swizzle, Type, ValidationError,
    ValidationErrorKind, wgsl,
};

fn float(program: &mut Program, value: f32) -> ExpressionId {
    program.push(Expression::Literal(Literal::Float(value)))
}

/// A valid vertex stage, so each test only breaks what it means to.
fn with_position() -> Program {
    let mut program = Program::new();
    let position = program.vertex_input("position", Type::Vec4f);
    let position = program.push(Expression::VertexInput(position));
    program.set_position(position);
    program
}

fn errors(program: &Program) -> Vec<ValidationError> {
    let errors = program.validate().expect_err("the program should be invalid");
    assert_eq!(wgsl::lower(program).expect_err("lowering validates"), errors);
    errors
}

fn kinds(program: &Program) -> Vec<ValidationErrorKind> {
    errors(program).into_iter().map(|error| error.kind).collect()
}

fn messages(program: &Program) -> Vec<String> {
    errors(program).iter().map(ToString::to_string).collect()
}

fn origin(line: u32, column: u32) -> SourceOrigin {
    SourceOrigin {
        owner: Arc::from("@test/viewer"),
        file: Arc::from("shaders/lit.jsl"),
        line,
        column,
    }
}

#[test]
fn dot_of_different_sizes_is_invalid() {
    let mut program = with_position();
    let normal = program.push(Expression::Literal(Literal::Vec3f([0.0, 0.0, 1.0])));
    let light = program.push(Expression::Literal(Literal::Vec4f([0.0, 1.0, 0.0, 0.0])));
    let facing = program.push_at(Expression::Dot(normal, light), origin(12, 5));
    let color = program.push(Expression::construct(Type::Vec4f, &[facing]).unwrap());
    program.push_fragment_output(color);

    assert_eq!(
        messages(&program),
        ["shaders/lit.jsl:12:5 (@test/viewer): dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type"]
    );
}

#[test]
fn sampling_with_vec3_coordinates_is_invalid() {
    let mut program = with_position();
    let texture = program.parameter("diffuse", Type::Texture2D);
    let sampler = program.parameter("diffuse", Type::Sampler);
    let texture = program.push(Expression::Parameter(texture));
    let sampler = program.push(Expression::Parameter(sampler));
    let coordinates = program.push(Expression::Literal(Literal::Vec3f([0.5, 0.5, 0.0])));
    let color = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates,
    });
    program.push_fragment_output(color);

    assert_eq!(
        messages(&program),
        ["sample(Texture2D, Sampler, Vec3f) is invalid: expected sample(Texture2D, Sampler, Vec2f)"]
    );
}

#[test]
fn a_texture_is_not_a_value() {
    let mut program = with_position();
    let texture = program.parameter("diffuse", Type::Texture2D);
    let texture = program.push(Expression::Parameter(texture));
    let one = float(&mut program, 1.0);
    let sum = program.push(Expression::Add(texture, one));
    program.push_fragment_output(sum);

    assert_eq!(
        messages(&program),
        [
            "Texture2D + Float is invalid: expected two values of the same numeric type, a vector and a scalar of its kind, \
             or two matrices of the same size"
        ]
    );
}

#[test]
fn stage_outputs_must_be_vec4f() {
    let mut program = Program::new();
    let position = program.vertex_input("position", Type::Vec3f);
    let position = program.push_at(Expression::VertexInput(position), origin(3, 1));
    program.set_position(position);
    let color = program.push(Expression::Literal(Literal::Vec3f([1.0, 0.0, 0.0])));
    program.push_fragment_output(color);

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::PositionType { found: Type::Vec3f },
            ValidationErrorKind::FragmentOutputType {
                location: 0,
                found: Type::Vec3f
            },
        ]
    );
    assert_eq!(errors(&program)[0].origin, Some(origin(3, 1)));
}

#[test]
fn a_program_needs_a_position() {
    let program = Program::new();
    assert_eq!(kinds(&program), [ValidationErrorKind::MissingPosition]);
}

#[test]
fn undefined_references_are_reported() {
    let mut other = Program::new();
    other.parameter("color", Type::Vec4f);
    let foreign_parameter = other.parameter("tint", Type::Vec4f);
    other.vertex_input("position", Type::Vec4f);
    let foreign_input = other.vertex_input("normal", Type::Vec4f);
    let foreign_varying = other.varying("uv", Type::Vec4f);

    let mut program = with_position();
    let parameter = program.push(Expression::Parameter(foreign_parameter));
    program.push_fragment_output(parameter);
    let input = program.push(Expression::VertexInput(foreign_input));
    program.write_varying(foreign_varying, input);

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::UndefinedVarying {
                expression: input,
                varying: foreign_varying
            },
            ValidationErrorKind::UndefinedParameter {
                expression: parameter,
                parameter: foreign_parameter
            },
            ValidationErrorKind::UndefinedVertexInput {
                expression: input,
                input: foreign_input
            },
        ]
    );
}

#[test]
fn expressions_from_another_program_are_reported() {
    let mut other = Program::new();
    let mut last = float(&mut other, 1.0);
    for _ in 0..8 {
        last = other.push(Expression::Add(last, last));
    }

    let mut program = with_position();
    let normalized = program.push(Expression::Normalize(last));
    program.push_fragment_output(normalized);
    program.push_fragment_output(last);

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::UndefinedExpression { expression: last },
            ValidationErrorKind::InvalidOperand {
                expression: normalized,
                operand: last
            },
        ]
    );
}

#[test]
fn the_stage_interface_is_checked() {
    let mut program = with_position();
    let uv = program.varying_at("uv", Type::Vec2f, origin(4, 7));
    let normal = program.varying("normal", Type::Vec3f);
    let tint = program.varying("tint", Type::Vec4f);
    let wrong = program.push(Expression::Literal(Literal::Vec4f([0.0; 4])));
    program.write_varying(normal, wrong);
    program.write_varying(tint, wrong);
    program.write_varying(tint, wrong);
    let uv = program.push(Expression::Varying(uv));
    let normal = program.push(Expression::Varying(normal));
    let zero = float(&mut program, 0.0);
    let first = program.push(Expression::construct(Type::Vec4f, &[uv, zero, zero]).unwrap());
    let second = program.push(Expression::construct(Type::Vec4f, &[normal, zero]).unwrap());
    program.push_fragment_output(first);
    program.push_fragment_output(second);

    let errors = errors(&program);
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "varying \"tint\" is written twice",
            "varying \"normal\" is declared Vec3f but written Vec4f",
            "shaders/lit.jsl:4:7 (@test/viewer): varying \"uv\" is read by the fragment stage but never written by \
             the vertex stage",
        ]
    );
}

#[test]
fn inputs_belong_to_their_stage() {
    let mut program = Program::new();
    let position = program.vertex_input("position", Type::Vec4f);
    let uv = program.varying("uv", Type::Vec2f);
    let texture = program.parameter("diffuse", Type::Texture2D);
    let sampler = program.parameter("diffuse", Type::Sampler);
    let position = program.push(Expression::VertexInput(position));
    let texture = program.push(Expression::Parameter(texture));
    let sampler = program.push(Expression::Parameter(sampler));
    let read_uv = program.push(Expression::Varying(uv));
    let texel = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates: read_uv,
    });
    let displaced = program.push(Expression::Add(position, texel));
    program.set_position(displaced);
    program.write_varying(uv, read_uv);
    program.push_fragment_output(position);

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::VertexInputInFragment {
                expression: position,
                input: "position".to_owned()
            },
            ValidationErrorKind::VaryingInVertex {
                expression: read_uv,
                varying: "uv".to_owned()
            },
        ]
    );

    let mut program = with_position();
    let texture = program.parameter("diffuse", Type::Texture2D);
    let sampler = program.parameter("diffuse", Type::Sampler);
    let texture = program.push(Expression::Parameter(texture));
    let sampler = program.push(Expression::Parameter(sampler));
    let coordinates = program.push(Expression::Literal(Literal::Vec2f([0.5, 0.5])));
    let texel = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates,
    });
    let tint = program.varying("tint", Type::Vec4f);
    program.write_varying(tint, texel);
    assert_eq!(
        messages(&program),
        ["sample is used in the vertex stage: textures are sampled in the fragment stage"]
    );
}

#[test]
fn pass_semantics_are_not_lowered_yet() {
    let mut program = with_position();
    let scene = program.push_at(Expression::Semantic(Semantic::SceneColor), origin(20, 3));
    let sampler = program.parameter("scene", Type::Sampler);
    let sampler = program.push(Expression::Parameter(sampler));
    let coordinates = program.push(Expression::Literal(Literal::Vec2f([0.5, 0.5])));
    let color = program.push(Expression::Sample {
        texture: scene,
        sampler,
        coordinates,
    });
    program.push_fragment_output(color);

    assert_eq!(
        messages(&program),
        ["shaders/lit.jsl:20:3 (@test/viewer): semantic sceneColor is not supported by the WGSL lowering yet"]
    );
}

#[test]
fn declarations_are_checked() {
    let mut program = with_position();
    program.parameter("1st", Type::Float);
    program.parameter("tint", Type::Vec4f);
    program.parameter("tint", Type::Vec3f);
    program.parameter("tint", Type::Texture2D);
    program.parameter("visible", Type::Bool);
    program.vertex_input("diffuse", Type::Texture2D);
    program.varying("transform", Type::Mat4f);
    program.varying("light-direction", Type::Vec3f);

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::InvalidName {
                kind: DeclarationKind::Parameter,
                name: "1st".to_owned()
            },
            ValidationErrorKind::DuplicateName {
                kind: DeclarationKind::Parameter,
                name: "tint".to_owned()
            },
            ValidationErrorKind::UnsupportedDeclarationType {
                kind: DeclarationKind::Parameter,
                name: "visible".to_owned(),
                ty: Type::Bool,
            },
            ValidationErrorKind::UnsupportedDeclarationType {
                kind: DeclarationKind::VertexInput,
                name: "diffuse".to_owned(),
                ty: Type::Texture2D,
            },
            ValidationErrorKind::UnsupportedDeclarationType {
                kind: DeclarationKind::Varying,
                name: "transform".to_owned(),
                ty: Type::Mat4f,
            },
            ValidationErrorKind::InvalidName {
                kind: DeclarationKind::Varying,
                name: "light-direction".to_owned()
            },
        ]
    );
}

#[test]
fn literals_wgsl_cannot_write_are_invalid() {
    let mut program = with_position();
    let not_a_number = float(&mut program, f32::NAN);
    let largest = program.push(Expression::Literal(Literal::Int(i32::MAX)));
    let one = program.push(Expression::Literal(Literal::Int(1)));
    let zero = program.push(Expression::Literal(Literal::UInt(0)));
    let seven = program.push(Expression::Literal(Literal::UInt(7)));
    let huge = float(&mut program, 3.0e38);
    let overflow = program.push(Expression::Add(largest, one));
    let division = program.push(Expression::Divide(seven, zero));
    let infinite = program.push(Expression::Multiply(huge, huge));
    let valid = program.push(Expression::Divide(largest, one));
    let written = [
        (not_a_number, Type::Float),
        (overflow, Type::Int),
        (division, Type::UInt),
        (infinite, Type::Float),
        (valid, Type::Int),
    ];
    for (index, (expression, ty)) in written.into_iter().enumerate() {
        let varying = program.varying(&format!("value{index}"), ty);
        program.write_varying(varying, expression);
    }

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::NonFiniteLiteral {
                expression: not_a_number
            },
            ValidationErrorKind::ConstantOutOfRange { expression: overflow },
            ValidationErrorKind::ConstantOutOfRange { expression: division },
            ValidationErrorKind::ConstantOutOfRange { expression: infinite },
        ]
    );
}

#[test]
fn operand_rules_are_checked() {
    let mut program = with_position();
    let scalar = float(&mut program, 1.0);
    let integer = program.push(Expression::Literal(Literal::Int(1)));
    let pair = program.push(Expression::Literal(Literal::Vec2f([1.0, 2.0])));
    let triple = program.push(Expression::Literal(Literal::Vec3f([1.0, 2.0, 3.0])));
    let quad = program.push(Expression::Literal(Literal::Vec4f([1.0, 2.0, 3.0, 4.0])));
    let model = program.push(Expression::Semantic(Semantic::Model));
    let expressions = [
        Expression::Min(integer, scalar),
        Expression::Multiply(model, triple),
        Expression::Divide(model, model),
        Expression::Cross(quad, quad),
        Expression::Mix(pair, triple, scalar),
        Expression::Clamp(triple, pair, scalar),
        Expression::Length(integer),
        Expression::Floor(integer),
        Expression::Normalize(scalar),
        Expression::construct(Type::Vec4f, &[triple, pair]).unwrap(),
        Expression::construct(Type::Mat3f, &[triple, triple, triple]).unwrap(),
        Expression::Swizzle {
            vector: pair,
            swizzle: Swizzle::parse("xyz").unwrap(),
        },
        Expression::Swizzle {
            vector: scalar,
            swizzle: Swizzle::parse("x").unwrap(),
        },
    ];
    for expression in expressions {
        let invalid = program.push(expression);
        let sink = program.varying(&format!("sink{}", invalid.index()), Type::Float);
        program.write_varying(sink, invalid);
    }

    assert_eq!(
        messages(&program),
        [
            "min(Int, Float) is invalid: expected two values of the same numeric type, or a vector and a scalar of its kind",
            "Mat4f * Vec3f is invalid: expected two values of the same numeric type, a vector and a scalar of its kind, a \
             matrix and a Float, a matrix and a vector of its size, or two matrices of the same size",
            "Mat4f / Mat4f is invalid: expected two values of the same numeric type, or a vector and a scalar of its kind",
            "cross(Vec4f, Vec4f) is invalid: expected two Vec3f",
            "mix(Vec2f, Vec3f, Float) is invalid: expected two Floats or float vectors of the same type, and an amount \
             of that type or Float",
            "clamp(Vec3f, Vec2f, Float) is invalid: expected a numeric value and two bounds of its type, or a vector \
             and two scalars of its kind",
            "length(Int) is invalid: expected a Float or float vector",
            "floor(Int) is invalid: expected a Float or float vector",
            "normalize(Float) is invalid: expected a float vector",
            "Vec4f(Vec3f, Vec2f) is invalid: expected floats and float vectors adding up to 4 components, or one Float",
            "Mat3f(Vec3f, Vec3f, Vec3f) is invalid: expected a vector target",
            "Vec2f.xyz is invalid: expected components a Vec2f has",
            "Float.x is invalid: expected components a Float has",
        ]
    );
}

#[test]
fn every_independent_mistake_is_reported_once() {
    let mut program = Program::new();
    let normal = program.push(Expression::Literal(Literal::Vec3f([0.0, 0.0, 1.0])));
    let light = program.push(Expression::Literal(Literal::Vec4f([0.0, 1.0, 0.0, 0.0])));
    let facing = program.push(Expression::Dot(normal, light));
    // Everything downstream of the bad `dot` is untyped, not reported again.
    let doubled = program.push(Expression::Add(facing, facing));
    let color = program.push(Expression::construct(Type::Vec4f, &[doubled]).unwrap());
    program.push_fragment_output(color);
    let wrong = program.push(Expression::Literal(Literal::Vec2f([0.0, 0.0])));
    program.push_fragment_output(wrong);

    assert_eq!(errors(&program).len(), 3, "{:#?}", errors(&program));
    assert_eq!(
        kinds(&program)[0],
        ValidationErrorKind::MissingPosition,
        "a missing position does not stop the other checks"
    );
}

#[test]
fn unused_expressions_are_not_checked() {
    let mut program = with_position();
    let normal = program.push(Expression::Literal(Literal::Vec3f([0.0, 0.0, 1.0])));
    let light = program.push(Expression::Literal(Literal::Vec4f([0.0, 1.0, 0.0, 0.0])));
    program.push(Expression::Dot(normal, light));
    assert_eq!(program.validate(), Ok(()));
}

#[test]
fn fragment_outputs_are_capped() {
    let mut program = with_position();
    let color = program.push(Expression::Literal(Literal::Vec4f([1.0; 4])));
    for _ in 0..=jess::shader::MAX_FRAGMENT_OUTPUTS {
        program.push_fragment_output(color);
    }
    assert_eq!(messages(&program), ["the fragment stage has 9 outputs: at most 8"]);
}
