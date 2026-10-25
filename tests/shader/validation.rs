//! Validation reports mistakes in Jess terms before any backend sees the program.

use std::sync::Arc;

use super::common;

use jess::shader::{
    Attribute, BinaryOperator, DataType, DeclarationKind, Element, Expression, ExpressionId, Field, Frequency, Function,
    Literal, Place, PlaceProblem, PlaceRoot, Program, Rate, Resource, ResourceId, ResourceKind, SampleLevel, SourceOrigin,
    Step, StructDecl, Swizzle, Type, Uniform, ValidationError, ValidationErrorKind, wgsl,
};

fn float(program: &mut Program, value: f32) -> ExpressionId {
    program.push(Expression::Literal(Literal::Float(value)))
}

/// Declares a material texture `name` and the sampler declared with it.
fn declare_texture(program: &mut Program, name: &str) -> (ResourceId, ResourceId) {
    let texture = program.resource(Resource::new(Frequency::Material, name, ResourceKind::Texture2D));
    let sampler = program.resource(Resource::new(Frequency::Material, name, ResourceKind::Sampler));
    (texture, sampler)
}

/// A valid vertex stage, so each test only breaks what it means to.
fn with_position() -> Program {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec4f));
    let position = program.push(Expression::Attribute(position));
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
    let facing = program.push_at(Expression::call(Function::Dot, &[normal, light]).unwrap(), origin(12, 5));
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
    let (texture, sampler) = declare_texture(&mut program, "diffuse");
    let coordinates = program.push(Expression::Literal(Literal::Vec3f([0.5, 0.5, 0.0])));
    let color = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates,
        layer: None,
        level: SampleLevel::Implicit,
    });
    program.push_fragment_output(color);

    assert_eq!(
        messages(&program),
        ["sample(Texture2D, Sampler, Vec3f) is invalid: expected sample(Texture2D, Sampler, Vec2f)"]
    );
}

#[test]
fn sampling_takes_a_texture_then_a_sampler() {
    let mut program = with_position();
    let (texture, sampler) = declare_texture(&mut program, "diffuse");
    let coordinates = program.push(Expression::Literal(Literal::Vec2f([0.5, 0.5])));
    let color = program.push(Expression::Sample {
        texture: sampler,
        sampler: texture,
        coordinates,
        layer: None,
        level: SampleLevel::Implicit,
    });
    program.push_fragment_output(color);

    assert_eq!(
        messages(&program),
        ["sample(Sampler, Texture2D, Vec2f) is invalid: expected sample(a texture, a Sampler, its coordinates)"]
    );
}

#[test]
fn a_load_leads_to_a_value() {
    let mut program = with_position();
    let light = program.structure(StructDecl::new(
        "Light",
        vec![Field::new("color", DataType::Value(Type::Vec3f))],
    ));
    program.uniform(Uniform::new(Frequency::Pass, "light", DataType::Struct(light)));
    let weights = DataType::Array {
        element: Element::Value(Type::Vec4f),
        length: 4,
    };
    program.uniform(Uniform::new(Frequency::Pass, "weights", weights));
    let eight = program.push(Expression::Literal(Literal::UInt(8)));
    let half = float(&mut program, 0.5);
    let place = |steps: &[Step]| Place::new(PlaceRoot::Uniform(Frequency::Pass), steps).unwrap();
    let places = [
        place(&[]),
        place(&[Step::Field(0)]),
        place(&[Step::Field(0), Step::Field(1)]),
        place(&[Step::Field(0), Step::Index(eight)]),
        place(&[Step::Field(1), Step::Field(0)]),
        place(&[Step::Field(1), Step::Index(eight)]),
        place(&[Step::Field(1), Step::Index(half)]),
        place(&[Step::Field(2)]),
    ];
    for place in places {
        let load = program.push(Expression::Load(place));
        program.push_fragment_output(load);
    }

    assert_eq!(
        messages(&program),
        [
            "a load names a uniform block, not one of its members",
            "a load ends at a struct or an array, not a value: read one of its fields or elements",
            "a load reads field 1 of Light, which it doesn't have",
            "a load indexes something that is not an array",
            "a load takes a field of something that is not a struct",
            "a load reads element 8 of an array of 4",
            "a load indexes an array with a Float: expected an Int or UInt",
            "a load reads member 2 of the pass uniforms, which is not declared",
        ]
    );
}

#[test]
fn stage_outputs_must_be_vec4f() {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec3f));
    let position = program.push_at(Expression::Attribute(position), origin(3, 1));
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
    common::uniform(&mut other, Frequency::Material, "color", Type::Vec4f);
    let foreign_uniform = common::uniform(&mut other, Frequency::Material, "tint", Type::Vec4f);
    let (foreign_texture, _) = declare_texture(&mut other, "diffuse");
    other.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec4f));
    let foreign_input = other.attribute(Attribute::new(Rate::Vertex, "normal", Type::Vec4f));
    let foreign_varying = other.varying("uv", Type::Vec4f);

    let mut program = with_position();
    let uniform = program.push(Expression::Load(foreign_uniform));
    program.push_fragment_output(uniform);
    let coordinates = program.push(Expression::Literal(Literal::Vec2f([0.5, 0.5])));
    let texel = program.push(Expression::Sample {
        texture: foreign_texture,
        sampler: foreign_texture,
        coordinates,
        layer: None,
        level: SampleLevel::Implicit,
    });
    program.push_fragment_output(texel);
    let input = program.push(Expression::Attribute(foreign_input));
    program.write_varying(foreign_varying, input);

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::UndefinedVarying {
                expression: input,
                varying: foreign_varying
            },
            ValidationErrorKind::InvalidPlace {
                expression: uniform,
                problem: PlaceProblem::UndefinedMember {
                    frequency: Frequency::Material,
                    member: 1
                }
            },
            ValidationErrorKind::UndefinedResource {
                expression: texel,
                resource: foreign_texture
            },
            ValidationErrorKind::UndefinedAttribute {
                expression: input,
                attribute: foreign_input
            },
        ]
    );
}

#[test]
fn expressions_from_another_program_are_reported() {
    let mut other = Program::new();
    let mut last = float(&mut other, 1.0);
    for _ in 0..8 {
        last = other.push(Expression::Binary(BinaryOperator::Add, last, last));
    }

    let mut program = with_position();
    let normalized = program.push(Expression::call(Function::Normalize, &[last]).unwrap());
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
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec4f));
    let uv = program.varying("uv", Type::Vec2f);
    let (texture, sampler) = declare_texture(&mut program, "diffuse");
    let position = program.push(Expression::Attribute(position));
    let read_uv = program.push(Expression::Varying(uv));
    let texel = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates: read_uv,
        layer: None,
        level: SampleLevel::Implicit,
    });
    let displaced = program.push(Expression::Binary(BinaryOperator::Add, position, texel));
    program.set_position(displaced);
    program.write_varying(uv, read_uv);
    program.push_fragment_output(position);

    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::AttributeInFragment {
                expression: position,
                attribute: "position".to_owned()
            },
            ValidationErrorKind::VaryingInVertex {
                expression: read_uv,
                varying: "uv".to_owned()
            },
        ]
    );

    let mut program = with_position();
    let (texture, sampler) = declare_texture(&mut program, "diffuse");
    let coordinates = program.push(Expression::Literal(Literal::Vec2f([0.5, 0.5])));
    let texel = program.push(Expression::Sample {
        texture,
        sampler,
        coordinates,
        layer: None,
        level: SampleLevel::Implicit,
    });
    let tint = program.varying("tint", Type::Vec4f);
    program.write_varying(tint, texel);
    assert_eq!(
        messages(&program),
        [
            "sample is used in the vertex stage, which has no derivatives to pick a mip level with: sample at a level with sampleLevel"
        ]
    );
}

#[test]
fn declarations_are_checked() {
    // Past the two structs the program declares: an id the program doesn't have.
    let mut other = Program::new();
    let density = || vec![Field::new("density", DataType::Value(Type::Float))];
    other.structure(StructDecl::new("Haze", density()));
    other.structure(StructDecl::new("Mist", density()));
    let foreign = other.structure(StructDecl::new("Fog", density()));

    let mut program = with_position();
    program.structure(StructDecl::new("Empty", Vec::new()));
    let color = Field::new("color", DataType::Value(Type::Vec3f));
    program.structure(StructDecl::new("Light", vec![color.clone(), color]));
    program.attribute(Attribute::new(Rate::Vertex, "flag", Type::Bool));
    let value = |ty: Type| DataType::Value(ty);
    let declared = [
        (Frequency::Material, "1st", value(Type::Float)),
        (Frequency::Material, "tint", value(Type::Vec4f)),
        (Frequency::Material, "tint", value(Type::Vec3f)),
        (Frequency::Material, "visible", value(Type::Bool)),
        (Frequency::Pass, "tint", value(Type::Vec4f)),
        (
            Frequency::Material,
            "lights",
            DataType::Array {
                element: Element::Value(Type::Vec4f),
                length: 0,
            },
        ),
        (Frequency::Material, "fog", DataType::Struct(foreign)),
        (
            Frequency::Material,
            "weights",
            DataType::Array {
                element: Element::Value(Type::Float),
                length: 4,
            },
        ),
    ];
    for (frequency, name, ty) in declared {
        program.uniform(Uniform::new(frequency, name, ty));
    }
    program.resource(Resource::new(Frequency::Material, "tint", ResourceKind::Texture2D));
    declare_texture(&mut program, "diffuse");
    program.varying("transform", Type::Mat4f);
    program.varying("light-direction", Type::Vec3f);

    let material = DeclarationKind::Uniform(Frequency::Material);
    assert_eq!(
        kinds(&program),
        [
            ValidationErrorKind::EmptyStruct {
                name: "Empty".to_owned()
            },
            ValidationErrorKind::DuplicateName {
                kind: DeclarationKind::Field,
                name: "color".to_owned()
            },
            ValidationErrorKind::UnsupportedDeclarationType {
                kind: DeclarationKind::Attribute(Rate::Vertex),
                name: "flag".to_owned(),
                ty: Type::Bool,
            },
            ValidationErrorKind::InvalidName {
                kind: material,
                name: "1st".to_owned()
            },
            ValidationErrorKind::DuplicateName {
                kind: material,
                name: "tint".to_owned()
            },
            ValidationErrorKind::UnsupportedDeclarationType {
                kind: material,
                name: "visible".to_owned(),
                ty: Type::Bool,
            },
            ValidationErrorKind::EmptyArray {
                kind: material,
                name: "lights".to_owned()
            },
            ValidationErrorKind::UndefinedStruct {
                kind: material,
                name: "fog".to_owned(),
                structure: foreign
            },
            ValidationErrorKind::DuplicateName {
                kind: DeclarationKind::Texture(Frequency::Material),
                name: "tint".to_owned()
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
            ValidationErrorKind::UniformArrayStride {
                path: "material.weights".to_owned(),
                stride: 4
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
    let overflow = program.push(Expression::Binary(BinaryOperator::Add, largest, one));
    let division = program.push(Expression::Binary(BinaryOperator::Divide, seven, zero));
    let infinite = program.push(Expression::Binary(BinaryOperator::Multiply, huge, huge));
    let valid = program.push(Expression::Binary(BinaryOperator::Divide, largest, one));
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
    let model = common::uniform(&mut program, Frequency::Draw, "model", Type::Mat4f);
    let model = program.push(Expression::Load(model));
    let expressions = [
        Expression::call(Function::Min, &[integer, scalar]).unwrap(),
        Expression::Binary(BinaryOperator::Multiply, model, triple),
        Expression::Binary(BinaryOperator::Divide, model, model),
        Expression::call(Function::Cross, &[quad, quad]).unwrap(),
        Expression::call(Function::Mix, &[pair, triple, scalar]).unwrap(),
        Expression::call(Function::Clamp, &[triple, pair, scalar]).unwrap(),
        Expression::call(Function::Length, &[integer]).unwrap(),
        Expression::call(Function::Floor, &[integer]).unwrap(),
        Expression::call(Function::Normalize, &[scalar]).unwrap(),
        Expression::construct(Type::Vec4f, &[triple, pair]).unwrap(),
        Expression::construct(Type::Float, &[triple]).unwrap(),
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
            "Float(Vec3f) is invalid: expected a vector or matrix target",
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
    let facing = program.push(Expression::call(Function::Dot, &[normal, light]).unwrap());
    // Everything downstream of the bad `dot` is untyped, not reported again.
    let doubled = program.push(Expression::Binary(BinaryOperator::Add, facing, facing));
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
    program.push(Expression::call(Function::Dot, &[normal, light]).unwrap());
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
