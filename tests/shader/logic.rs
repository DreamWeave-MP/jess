//! Comparisons, logic, and `select`: scalars compared to a `Bool`, `Bool`s combined, and a value
//! chosen by one, all checked by naga.

use super::common::assert_valid_wgsl;

use jess::shader::wgsl;
use jess::shader::{BinaryOperator, Literal, ProgramBuilder, Rate, Type, UnaryOperator};

#[test]
fn comparisons_choose_between_values() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let shade = site.attribute(Rate::Vertex, "shade", Type::Float).unwrap();
    let shade = site.read_attribute(shade);
    let layer = site.attribute(Rate::Vertex, "layer", Type::UInt).unwrap();
    let layer = site.read_attribute(layer);
    site.set_position(position).unwrap();
    let shade = site.pass("shade", shade).unwrap();
    let layer = site.pass("layer", layer).unwrap();

    let half = site.float(0.5).unwrap();
    let dark = site.binary(BinaryOperator::Less, shade, half).unwrap();
    assert_eq!(dark.ty(), Type::Bool);
    let two = site.uint(2);
    let top = site.binary(BinaryOperator::GreaterEqual, layer, two).unwrap();
    let lit = site.unary(UnaryOperator::Not, dark).unwrap();
    let either = site.binary(BinaryOperator::Or, lit, top).unwrap();
    let both = site.binary(BinaryOperator::And, either, dark).unwrap();
    let same = site.binary(BinaryOperator::Equal, both, either).unwrap();
    let red = site.literal(Literal::Vec4f([1.0, 0.0, 0.0, 1.0])).unwrap();
    let blue = site.literal(Literal::Vec4f([0.0, 0.0, 1.0, 1.0])).unwrap();
    let color = site.select(same, red, blue).unwrap();
    assert_eq!(color.ty(), Type::Vec4f);
    site.output(color).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "let e0 = input.v_shade < 0.5;",
        "let e1 = input.v_layer >= 2u;",
        "let e2 = !e0;",
        "let e3 = e2 || e1;",
        "let e4 = e3 && e0;",
        "let e5 = e4 == e3;",
        "let e6 = select(vec4<f32>(0.0, 0.0, 1.0, 1.0), vec4<f32>(1.0, 0.0, 0.0, 1.0), e5);",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn comparisons_take_scalars_of_one_type() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let half = site.float(0.5).unwrap();
    let one = site.int(1);
    let on = site.bool(true);
    let pair = site.literal(Literal::Vec2f([1.0, 2.0])).unwrap();

    let refusals = [
        (
            site.binary(BinaryOperator::Less, half, one).unwrap_err(),
            "less(Float, Int) is invalid: expected two Ints, UInts, or Floats of the same type",
        ),
        (
            site.binary(BinaryOperator::Less, on, on).unwrap_err(),
            "less(Bool, Bool) is invalid: expected two Ints, UInts, or Floats of the same type",
        ),
        (
            site.binary(BinaryOperator::Equal, pair, pair).unwrap_err(),
            "equal(Vec2f, Vec2f) is invalid: expected two scalars of the same type",
        ),
        (
            site.binary(BinaryOperator::And, on, half).unwrap_err(),
            "logicalAnd(Bool, Float) is invalid: expected two Bools",
        ),
        (
            site.unary(UnaryOperator::Not, one).unwrap_err(),
            "logicalNot(Int) is invalid: expected a Bool",
        ),
        (
            site.select(half, pair, pair).unwrap_err(),
            "select(Float, Vec2f, Vec2f) is invalid: expected a Bool, then two scalars or vectors of the same type",
        ),
        (
            site.select(on, pair, half).unwrap_err(),
            "select(Bool, Vec2f, Float) is invalid: expected a Bool, then two scalars or vectors of the same type",
        ),
    ];
    for (error, message) in refusals {
        assert_eq!(error.to_string(), message);
    }
}
