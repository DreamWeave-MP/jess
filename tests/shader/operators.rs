//! Negation, remainder, and the bitwise operators, checked by naga, with the constant mistakes WGSL
//! would reject caught first.

use super::common::assert_valid_wgsl;

use jess::shader::wgsl;
use jess::shader::{BinaryOperator, Literal, ProgramBuilder, Rate, Type, UnaryOperator, ValidationErrorKind};

#[test]
fn negation_remainder_and_bits_lower() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let id = site.attribute(Rate::Vertex, "id", Type::UInt).unwrap();
    let id = site.read_attribute(id);
    let cell = site.attribute(Rate::Vertex, "cell", Type::Vec2i).unwrap();
    let cell = site.read_attribute(cell);

    let flipped = site.unary(UnaryOperator::Negate, position).unwrap();
    site.set_position(flipped).unwrap();
    let minus_one = site.float(-1.0).unwrap();
    let back = site.unary(UnaryOperator::Negate, minus_one).unwrap();
    site.pass("back", back).unwrap();

    let eight = site.uint(8);
    let low = site.uint(0xff);
    let high = site.binary(BinaryOperator::ShiftRight, id, eight).unwrap();
    let byte = site.binary(BinaryOperator::BitAnd, high, low).unwrap();
    let mixed = site.binary(BinaryOperator::BitXor, byte, id).unwrap();
    let inverted = site.unary(UnaryOperator::BitNot, mixed).unwrap();
    let one = site.uint(1);
    let raised = site.binary(BinaryOperator::ShiftLeft, inverted, one).unwrap();
    let packed = site.binary(BinaryOperator::BitOr, raised, one).unwrap();
    site.pass("packed", packed).unwrap();

    let four = site.int(4);
    let wrapped = site.binary(BinaryOperator::Remainder, cell, four).unwrap();
    assert_eq!(wrapped.ty(), Type::Vec2i);
    site.pass("wrapped", wrapped).unwrap();
    let amounts = site.construct(Type::Vec2u, &[one]).unwrap();
    let shifted = site.binary(BinaryOperator::ShiftLeft, cell, amounts).unwrap();
    site.pass("shifted", shifted).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "let e0 = -(input.a_position);",
        "let e1 = -(-1.0);",
        "let e2 = input.a_id >> 8u;",
        "let e3 = e2 & 255u;",
        "let e4 = e3 ^ input.a_id;",
        "let e5 = ~(e4);",
        "let e6 = e5 << 1u;",
        "let e7 = e6 | 1u;",
        "let e8 = input.a_cell % 4i;",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn the_operators_take_what_wgsl_takes() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let count = site.uint(3);
    let signed = site.int(3);
    let half = site.float(0.5).unwrap();
    let pair = site.literal(Literal::Vec2f([1.0, 2.0])).unwrap();

    let refusals = [
        (
            site.unary(UnaryOperator::Negate, count).unwrap_err(),
            "-(UInt) is invalid: expected a signed or float scalar or vector",
        ),
        (
            site.unary(UnaryOperator::BitNot, half).unwrap_err(),
            "bitNot(Float) is invalid: expected an Int or UInt value",
        ),
        (
            site.binary(BinaryOperator::BitAnd, signed, count).unwrap_err(),
            "bitAnd(Int, UInt) is invalid: expected two Int or UInt values of the same type",
        ),
        (
            site.binary(BinaryOperator::ShiftLeft, signed, signed).unwrap_err(),
            "shiftLeft(Int, Int) is invalid: expected an Int or UInt value, and a UInt amount of its size",
        ),
        (
            site.binary(BinaryOperator::Remainder, pair, signed).unwrap_err(),
            "Vec2f % Int is invalid: expected two values of the same numeric type, or a vector and a scalar of its kind",
        ),
    ];
    for (error, message) in refusals {
        assert_eq!(error.to_string(), message);
    }

    let zero = site.int(0);
    let by_zero = site.binary(BinaryOperator::Remainder, signed, zero).unwrap_err();
    assert!(
        matches!(by_zero.kind, ValidationErrorKind::ConstantOutOfRange { .. }),
        "{by_zero:?}"
    );
    let far = site.uint(32);
    let too_far = site.binary(BinaryOperator::ShiftLeft, count, far).unwrap_err();
    assert_eq!(too_far.to_string(), "a shift by 32 is too far: shift by less than 32");
}
