//! Trivial constant folding: constant arithmetic, constant vectors, and identities. Nothing else.

use super::common::uniform;

use jess::shader::{
    Attribute, BinaryOperator, Expression, ExpressionId, Frequency, Literal, Program, Rate, Type, ValidationErrorKind,
};

fn literal(program: &mut Program, literal: Literal) -> ExpressionId {
    program.push(Expression::Literal(literal))
}

/// A program whose single fragment output is `output(program)`, with a pass-through position.
fn fragment(output: impl FnOnce(&mut Program) -> ExpressionId) -> Program {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec4f));
    let position = program.push(Expression::Attribute(position));
    program.set_position(position);
    let color = output(&mut program);
    program.push_fragment_output(color);
    program
}

fn output(program: &Program) -> Expression {
    *program.expression(program.fragment_outputs()[0]).unwrap()
}

#[test]
fn constant_arithmetic_folds() {
    let mut program = fragment(|program| {
        let two = literal(program, Literal::Float(2.0));
        let three = literal(program, Literal::Float(3.0));
        let six = program.push(Expression::Binary(BinaryOperator::Multiply, two, three));
        let half = literal(program, Literal::Float(0.5));
        let sum = program.push(Expression::Binary(BinaryOperator::Add, six, half));
        let base = literal(program, Literal::Vec4f([1.0, 2.0, 3.0, 4.0]));
        program.push(Expression::Binary(BinaryOperator::Divide, base, sum))
    });
    assert_eq!(program.fold_constants(), Ok(3));
    let quotient = [1.0 / 6.5, 2.0 / 6.5, 3.0 / 6.5, 4.0 / 6.5];
    assert_eq!(output(&program), Expression::Literal(Literal::Vec4f(quotient)));
}

#[test]
fn integer_arithmetic_folds() {
    let mut program = Program::new();
    let position = program.attribute(Attribute::new(Rate::Vertex, "position", Type::Vec4f));
    let position = program.push(Expression::Attribute(position));
    program.set_position(position);
    let seven = literal(&mut program, Literal::Int(-7));
    let two = literal(&mut program, Literal::Int(2));
    let quotient = program.push(Expression::Binary(BinaryOperator::Divide, seven, two));
    let count = program.varying("count", Type::Int);
    program.write_varying(count, quotient);
    let large = literal(&mut program, Literal::UInt(4_000_000_000));
    let small = literal(&mut program, Literal::UInt(5));
    let difference = program.push(Expression::Binary(BinaryOperator::Subtract, large, small));
    let index = program.varying("index", Type::UInt);
    program.write_varying(index, difference);

    assert_eq!(program.fold_constants(), Ok(2));
    assert_eq!(program.expression(quotient), Some(&Expression::Literal(Literal::Int(-3))));
    assert_eq!(
        program.expression(difference),
        Some(&Expression::Literal(Literal::UInt(3_999_999_995)))
    );
}

#[test]
fn constant_vectors_fold() {
    let mut program = fragment(|program| {
        let red = literal(program, Literal::Float(1.0));
        let green = literal(program, Literal::Vec2f([0.5, 0.25]));
        let alpha = literal(program, Literal::Float(1.0));
        program.push(Expression::construct(Type::Vec4f, &[red, green, alpha]).unwrap())
    });
    assert_eq!(program.fold_constants(), Ok(1));
    assert_eq!(output(&program), Expression::Literal(Literal::Vec4f([1.0, 0.5, 0.25, 1.0])));

    let mut program = fragment(|program| {
        let gray = literal(program, Literal::Float(0.5));
        program.push(Expression::construct(Type::Vec4f, &[gray]).unwrap())
    });
    assert_eq!(program.fold_constants(), Ok(1));
    assert_eq!(output(&program), Expression::Literal(Literal::Vec4f([0.5; 4])));
}

#[test]
fn a_float_against_a_vector_folds_per_component() {
    let mut program = fragment(|program| {
        let scale = literal(program, Literal::Float(2.0));
        let color = literal(program, Literal::Vec4f([0.25, 0.5, 1.0, 0.5]));
        program.push(Expression::Binary(BinaryOperator::Multiply, scale, color))
    });
    assert_eq!(program.fold_constants(), Ok(1));
    assert_eq!(output(&program), Expression::Literal(Literal::Vec4f([0.5, 1.0, 2.0, 1.0])));
}

/// Builds an operation from the value and the constant.
type Build = fn(ExpressionId, ExpressionId) -> Expression;

#[test]
fn identities_fold_to_their_operand() {
    let color = |program: &mut Program| {
        let color = uniform(program, Frequency::Material, "color", Type::Vec4f);
        program.push(Expression::Load(color))
    };
    let cases: [(Literal, Build, bool); 6] = [
        (
            Literal::Float(1.0),
            |value, one| Expression::Binary(BinaryOperator::Multiply, value, one),
            true,
        ),
        (
            Literal::Vec4f([1.0; 4]),
            |value, one| Expression::Binary(BinaryOperator::Multiply, one, value),
            true,
        ),
        (
            Literal::Float(1.0),
            |value, one| Expression::Binary(BinaryOperator::Divide, value, one),
            true,
        ),
        (
            Literal::Float(0.0),
            |value, zero| Expression::Binary(BinaryOperator::Add, zero, value),
            true,
        ),
        (
            Literal::Vec4f([0.0; 4]),
            |value, zero| Expression::Binary(BinaryOperator::Subtract, value, zero),
            true,
        ),
        // `0 - x` is not `x`.
        (
            Literal::Float(0.0),
            |value, zero| Expression::Binary(BinaryOperator::Subtract, zero, value),
            false,
        ),
    ];
    for (constant, build, folds) in cases {
        let mut kept = None;
        let mut program = fragment(|program| {
            let value = color(program);
            kept = Some(value);
            let constant = literal(program, constant);
            program.push(build(value, constant))
        });
        assert_eq!(program.fold_constants(), Ok(usize::from(folds)), "{constant:?}");
        assert_eq!(program.fragment_outputs()[0] == kept.unwrap(), folds, "{constant:?}");
    }
}

#[test]
fn an_identity_that_changes_the_type_does_not_fold() {
    let mut program = fragment(|program| {
        let transform = uniform(program, Frequency::Draw, "model", Type::Mat4f);
        let transform = program.push(Expression::Load(transform));
        let ones = literal(program, Literal::Vec4f([1.0; 4]));
        program.push(Expression::Binary(BinaryOperator::Multiply, transform, ones))
    });
    let product = program.fragment_outputs()[0];
    assert_eq!(program.fold_constants(), Ok(0));
    assert_eq!(program.fragment_outputs()[0], product);
}

#[test]
fn folding_reaches_through_forwarded_operands() {
    let mut program = fragment(|program| {
        let half = literal(program, Literal::Float(0.5));
        let one = literal(program, Literal::Float(1.0));
        let still_half = program.push(Expression::Binary(BinaryOperator::Multiply, half, one));
        program.push(Expression::construct(Type::Vec4f, &[still_half]).unwrap())
    });
    assert_eq!(program.fold_constants(), Ok(2));
    assert_eq!(output(&program), Expression::Literal(Literal::Vec4f([0.5; 4])));
    assert_eq!(program.fold_constants(), Ok(0), "folding twice changes nothing");
}

#[test]
fn an_invalid_program_is_left_alone() {
    let mut program = fragment(|program| {
        let two = literal(program, Literal::Float(2.0));
        let four = program.push(Expression::Binary(BinaryOperator::Multiply, two, two));
        program.push(Expression::Binary(BinaryOperator::Less, four, two))
    });
    let before = program.expressions().to_vec();
    let errors = program.fold_constants().unwrap_err();
    assert!(matches!(
        errors[0].kind,
        ValidationErrorKind::FragmentOutputType { found: Type::Bool, .. }
    ));
    assert_eq!(program.expressions(), before);
}
