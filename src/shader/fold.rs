//! Trivial constant folding: constant arithmetic, constant vector constructors, and `x * 1`,
//! `x / 1`, `x + 0`, `x - 0`. Nothing else: the backend compiler optimizes; Jess only avoids
//! handing it noise.

use crate::shader::expression::{BinaryOperator, Expression, ExpressionId, Literal};
use crate::shader::flow::Statement;
use crate::shader::program::Program;
use crate::shader::types::Type;
use crate::shader::validate::{ValidationError, analyze};

/// An arithmetic operator, for [`evaluate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
}

impl Arithmetic {
    pub(crate) fn of(expression: &Expression) -> Option<(Arithmetic, ExpressionId, ExpressionId)> {
        let Expression::Binary(operator, left, right) = *expression else {
            return None;
        };
        let arithmetic = match operator {
            BinaryOperator::Add => Arithmetic::Add,
            BinaryOperator::Subtract => Arithmetic::Subtract,
            BinaryOperator::Multiply => Arithmetic::Multiply,
            BinaryOperator::Divide => Arithmetic::Divide,
            BinaryOperator::Remainder => Arithmetic::Remainder,
            _ => return None,
        };
        Some((arithmetic, left, right))
    }
}

/// `left operator right` on two literals whose types the operator accepts; `None` if the result
/// is not representable: integer overflow, integer division by zero, or a non-finite float.
/// WGSL rejects exactly those as constant expressions.
pub fn evaluate(operator: Arithmetic, left: Literal, right: Literal) -> Option<Literal> {
    match (left, right) {
        (Literal::Int(left), Literal::Int(right)) => {
            let value = match operator {
                Arithmetic::Add => left.checked_add(right),
                Arithmetic::Subtract => left.checked_sub(right),
                Arithmetic::Multiply => left.checked_mul(right),
                Arithmetic::Divide => left.checked_div(right),
                Arithmetic::Remainder => left.checked_rem(right),
            };
            value.map(Literal::Int)
        }
        (Literal::UInt(left), Literal::UInt(right)) => {
            let value = match operator {
                Arithmetic::Add => left.checked_add(right),
                Arithmetic::Subtract => left.checked_sub(right),
                Arithmetic::Multiply => left.checked_mul(right),
                Arithmetic::Divide => left.checked_div(right),
                Arithmetic::Remainder => left.checked_rem(right),
            };
            value.map(Literal::UInt)
        }
        _ => {
            let (left, right) = (left.floats(), right.floats());
            let size = left.len().max(right.len());
            let mut values = [0.0f32; 4];
            for (index, value) in values[..size].iter_mut().enumerate() {
                // A `Float` against a vector repeats; validation guarantees the sizes otherwise match.
                let left = left[if left.len() == 1 { 0 } else { index }];
                let right = right[if right.len() == 1 { 0 } else { index }];
                *value = match operator {
                    Arithmetic::Add => left + right,
                    Arithmetic::Subtract => left - right,
                    Arithmetic::Multiply => left * right,
                    Arithmetic::Divide => left / right,
                    // Truncated, as WGSL's is: the sign of `left`.
                    Arithmetic::Remainder => left % right,
                };
                if !value.is_finite() {
                    return None;
                }
            }
            float_literal(&values[..size])
        }
    }
}

fn float_literal(values: &[f32]) -> Option<Literal> {
    match *values {
        [x] => Some(Literal::Float(x)),
        [x, y] => Some(Literal::Vec2f([x, y])),
        [x, y, z] => Some(Literal::Vec3f([x, y, z])),
        [x, y, z, w] => Some(Literal::Vec4f([x, y, z, w])),
        _ => None,
    }
}

#[allow(clippy::float_cmp)] // the identities hold for exactly 0 and 1 only
fn is_all(literal: Literal, value: u8) -> bool {
    match literal {
        Literal::Bool(_) => false,
        Literal::Int(integer) => integer == i32::from(value),
        Literal::UInt(integer) => integer == u32::from(value),
        _ => literal.floats().iter().all(|&float| float == f32::from(value)),
    }
}

impl Program {
    /// Folds constant arithmetic, vector constructors of constants, and the identities `x * 1`,
    /// `1 * x`, `x / 1`, `x + 0`, `0 + x`, and `x - 0` (where the result has `x`'s type), in the
    /// expressions a stage reads. Returns how many expressions were folded.
    ///
    /// Validates first and leaves the program untouched when it is invalid. A folded expression
    /// becomes a literal in place; an identity's readers read `x` instead. `x + 0` folds even
    /// though `-0 + 0` is `+0`: WGSL does not promise to keep the sign of zero either.
    ///
    /// # Errors
    ///
    /// The program's validation errors, as [`validate`](Program::validate) reports them.
    pub fn fold_constants(&mut self) -> Result<usize, Vec<ValidationError>> {
        let analysis = analyze(self)?;
        let mut forward: Vec<ExpressionId> = (0..self.expressions.len()).map(ExpressionId::at).collect();
        let mut folded = 0;
        for index in 0..self.expressions.len() {
            let Some(ty) = analysis.types[index] else {
                continue;
            };
            let mut expression = self.expressions[index];
            expression.rewrite_operands(|operand| *operand = forward[operand.index()]);
            match fold(&self.expressions, &analysis.types, &expression, ty) {
                Some(Folded::Literal(literal)) => {
                    expression = Expression::Literal(literal);
                    folded += 1;
                }
                Some(Folded::Forward(operand)) => {
                    forward[index] = operand;
                    folded += 1;
                }
                None => {}
            }
            self.expressions[index] = expression;
        }

        if let Some(position) = &mut self.position {
            *position = forward[position.index()];
        }
        for (_, expression) in &mut self.varying_writes {
            *expression = forward[expression.index()];
        }
        for output in &mut self.fragment_outputs {
            *output = forward[output.index()];
        }
        for body in &mut self.bodies {
            forward_statements(body, &forward);
        }
        Ok(folded)
    }
}

/// Points every value a statement reads at what it folded to. A placed expression stays placed, as
/// what it folded to is visible wherever it was.
fn forward_statements(block: &mut [Statement], forward: &[ExpressionId]) {
    for statement in block {
        match statement {
            Statement::Emit(_) | Statement::Discard { .. } => {}
            Statement::Store { value, .. } => *value = forward[value.index()],
            Statement::Write { place, value, .. } => {
                place.rewrite_indices(|index| *index = forward[index.index()]);
                *value = forward[value.index()];
            }
            Statement::If {
                condition,
                accept,
                reject,
                ..
            } => {
                *condition = forward[condition.index()];
                forward_statements(accept, forward);
                forward_statements(reject, forward);
            }
            Statement::Loop { count, body, .. } => {
                *count = forward[count.index()];
                forward_statements(body, forward);
            }
        }
    }
}

enum Folded {
    Literal(Literal),
    Forward(ExpressionId),
}

fn fold(expressions: &[Expression], types: &[Option<Type>], expression: &Expression, ty: Type) -> Option<Folded> {
    let literal_of = |id: ExpressionId| match expressions[id.index()] {
        Expression::Literal(literal) => Some(literal),
        _ => None,
    };

    if let Expression::Construct { components, .. } = expression {
        // Only a float vector folds: a matrix's columns would not fit, and integers stay as written.
        if !ty.is_float_vector() {
            return None;
        }
        let mut values = [0.0f32; 4];
        let mut size = 0;
        for &component in components.iter() {
            let literal = literal_of(component)?;
            let floats = literal.floats();
            values[size..size + floats.len()].copy_from_slice(floats);
            size += floats.len();
        }
        let target = ty.float_components() as usize;
        if size == 1 {
            values = [values[0]; 4];
        }
        return float_literal(&values[..target]).map(Folded::Literal);
    }

    let (operator, left, right) = Arithmetic::of(expression)?;
    let (left_literal, right_literal) = (literal_of(left), literal_of(right));
    if let (Some(left_literal), Some(right_literal)) = (left_literal, right_literal) {
        return evaluate(operator, left_literal, right_literal).map(Folded::Literal);
    }

    let keeps = |operand: ExpressionId| types[operand.index()] == Some(ty);
    let right_is = |value| right_literal.is_some_and(|literal| is_all(literal, value));
    let left_is = |value| left_literal.is_some_and(|literal| is_all(literal, value));
    let kept = match operator {
        Arithmetic::Add | Arithmetic::Subtract if right_is(0) && keeps(left) => left,
        Arithmetic::Add if left_is(0) && keeps(right) => right,
        Arithmetic::Multiply | Arithmetic::Divide if right_is(1) && keeps(left) => left,
        Arithmetic::Multiply if left_is(1) && keeps(right) => right,
        _ => return None,
    };
    Some(Folded::Forward(kept))
}
