//! A typed symbolic builder over the IR, for a frontend that constructs a program one expression
//! at a time and wants each mistake reported where the author wrote it.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::shader::expression::{Expression, ExpressionId, Literal, Semantic, SourceOrigin, Swizzle};
use crate::shader::program::{Declaration, Program};
use crate::shader::types::Type;
use crate::shader::validate::{self, DeclarationKind, ValidationError, ValidationErrorKind};

/// Numbers every builder, so a value knows which program it belongs to.
static NEXT_PROGRAM: AtomicU64 = AtomicU64::new(0);

/// A typed, symbolic value in one program under construction: an expression, its type, and the
/// program it belongs to.
///
/// Values are `Copy` and only usable with the builder that made them. A value is not a GPU
/// value: it names a node the program will compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Value {
    program: u64,
    expression: ExpressionId,
    ty: Type,
}

impl Value {
    /// The value's type.
    #[must_use]
    pub fn ty(self) -> Type {
        self.ty
    }

    /// The expression the value names, in its program's arena.
    #[must_use]
    pub fn expression(self) -> ExpressionId {
        self.expression
    }
}

/// Builds a [`Program`] one typed expression at a time, for a frontend that wants each mistake
/// reported where the author wrote it.
///
/// The builder owns the program under construction. Every declaration and operation goes
/// through a [`Site`], the builder positioned at the author's [`SourceOrigin`] (or at none), and
/// yields a [`Value`]: a `Copy` handle naming one expression, its [`Type`], and the program it
/// belongs to. Operand types are checked as each operation is pushed, by the same rules
/// [`Program::validate`] applies, so `dot(Vec3f, Vec4f)` fails at the call that wrote it, with
/// that call's origin. Rules that need the whole program (which stage reads what, the types of
/// the outputs, the pass-specific semantics the lowering cannot bind yet) run in
/// [`finish`](ProgramBuilder::finish), with the origins the sites recorded.
///
/// Varyings are not declared by hand. [`Site::pass`] declares one from a vertex value's name and
/// type, writes it, and returns the value the fragment stage reads, so a frontend whose vertex
/// function returns `{ position = ..., uv = ..., normal = ... }` passes each named field and
/// hands the results to the fragment function.
///
/// A value from another builder is refused wherever it is used: [`ExpressionId`]s alias across
/// programs, and a value carries the program it came from.
#[derive(Debug)]
pub struct ProgramBuilder {
    token: u64,
    program: Program,
}

impl Default for ProgramBuilder {
    fn default() -> ProgramBuilder {
        ProgramBuilder::new()
    }
}

impl ProgramBuilder {
    /// A builder for an empty program.
    #[must_use]
    pub fn new() -> ProgramBuilder {
        ProgramBuilder {
            token: NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed),
            program: Program::new(),
        }
    }

    /// The builder positioned at no source: for programs built natively.
    pub fn site(&mut self) -> Site<'_> {
        Site {
            builder: self,
            origin: None,
        }
    }

    /// The builder positioned at `origin`: everything declared or pushed through the site records
    /// it, and every error the site reports carries it.
    pub fn at(&mut self, origin: SourceOrigin) -> Site<'_> {
        Site {
            builder: self,
            origin: Some(origin),
        }
    }

    /// The program so far, for inspection.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Validates the program and returns it.
    ///
    /// # Errors
    ///
    /// Every problem [`Program::validate`] finds: the checks that need the whole program, such as
    /// a vertex input read in the fragment stage, a position that is not a `Vec4f`, or a semantic
    /// the lowering cannot bind yet.
    pub fn finish(self) -> Result<Program, Vec<ValidationError>> {
        self.program.validate()?;
        Ok(self.program)
    }
}

/// A [`ProgramBuilder`] positioned at one source site, from [`ProgramBuilder::site`] or
/// [`ProgramBuilder::at`]. Every method reports its error with the site's origin.
#[derive(Debug)]
pub struct Site<'a> {
    builder: &'a mut ProgramBuilder,
    origin: Option<SourceOrigin>,
}

impl Site<'_> {
    /// Declares a parameter (a value, a `Texture2D`, or a `Sampler`) and returns its value.
    ///
    /// # Errors
    ///
    /// The first problem with the declaration: an invalid name, a name already declared, or a
    /// type a parameter cannot have.
    pub fn parameter(&mut self, name: &str, ty: Type) -> Result<Value, ValidationError> {
        self.check_declaration(DeclarationKind::Parameter, &self.builder.program.parameters, name, ty)?;
        let program = &mut self.builder.program;
        let parameter = match &self.origin {
            Some(origin) => program.parameter_at(name, ty, origin.clone()),
            None => program.parameter(name, ty),
        };
        Ok(self.push(Expression::Parameter(parameter), ty))
    }

    /// Declares a per-vertex attribute and returns its value, which only the vertex stage may
    /// read.
    ///
    /// # Errors
    ///
    /// The first problem with the declaration: an invalid name, a name already declared, or a
    /// type a vertex input cannot have.
    pub fn vertex_input(&mut self, name: &str, ty: Type) -> Result<Value, ValidationError> {
        self.check_declaration(DeclarationKind::VertexInput, &self.builder.program.vertex_inputs, name, ty)?;
        let program = &mut self.builder.program;
        let input = match &self.origin {
            Some(origin) => program.vertex_input_at(name, ty, origin.clone()),
            None => program.vertex_input(name, ty),
        };
        Ok(self.push(Expression::VertexInput(input), ty))
    }

    /// Passes `value` from the vertex stage to the fragment stage as the varying `name`, declared
    /// with `value`'s type, and returns the value the fragment stage reads.
    ///
    /// # Errors
    ///
    /// A value from another program, or the first problem with the declaration: an invalid name,
    /// a name already passed, or a type a varying cannot have (a matrix or a resource).
    pub fn pass(&mut self, name: &str, value: Value) -> Result<Value, ValidationError> {
        self.own(value)?;
        self.check_declaration(DeclarationKind::Varying, &self.builder.program.varyings, name, value.ty)?;
        let program = &mut self.builder.program;
        let varying = match &self.origin {
            Some(origin) => program.varying_at(name, value.ty, origin.clone()),
            None => program.varying(name, value.ty),
        };
        program.write_varying(varying, value.expression);
        Ok(self.push(Expression::Varying(varying), value.ty))
    }

    /// A constant.
    ///
    /// # Errors
    ///
    /// A NaN or infinite float, which WGSL cannot write.
    pub fn literal(&mut self, literal: Literal) -> Result<Value, ValidationError> {
        let ty = validate::literal_type(self.next(), literal).map_err(|kind| self.error(kind))?;
        Ok(self.push(Expression::Literal(literal), ty))
    }

    /// A `Float` constant.
    ///
    /// # Errors
    ///
    /// A NaN or infinite value, which WGSL cannot write.
    pub fn float(&mut self, value: f32) -> Result<Value, ValidationError> {
        self.literal(Literal::Float(value))
    }

    /// An `Int` constant.
    pub fn int(&mut self, value: i32) -> Value {
        self.push(Expression::Literal(Literal::Int(value)), Type::Int)
    }

    /// A `UInt` constant.
    pub fn uint(&mut self, value: u32) -> Value {
        self.push(Expression::Literal(Literal::UInt(value)), Type::UInt)
    }

    /// A `Bool` constant.
    pub fn bool(&mut self, value: bool) -> Value {
        self.push(Expression::Literal(Literal::Bool(value)), Type::Bool)
    }

    /// A value the renderer provides. The pass-specific semantics (`sceneColor`, `sceneDepth`,
    /// `previousPass`) are accepted here and rejected by [`ProgramBuilder::finish`], as the
    /// lowering cannot bind them yet.
    pub fn semantic(&mut self, semantic: Semantic) -> Value {
        self.push(Expression::Semantic(semantic), semantic.ty())
    }

    /// `left + right`.
    ///
    /// # Errors
    ///
    /// A value from another program, operand types the operation does not accept, or constant
    /// arithmetic WGSL would reject.
    pub fn add(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Add(left.expression, right.expression), &[left, right])
    }

    /// `left - right`.
    ///
    /// # Errors
    ///
    /// As [`add`](Site::add).
    pub fn subtract(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Subtract(left.expression, right.expression), &[left, right])
    }

    /// `left * right`, including matrix × vector and matrix × matrix.
    ///
    /// # Errors
    ///
    /// As [`add`](Site::add).
    pub fn multiply(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Multiply(left.expression, right.expression), &[left, right])
    }

    /// `left / right`.
    ///
    /// # Errors
    ///
    /// As [`add`](Site::add).
    pub fn divide(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Divide(left.expression, right.expression), &[left, right])
    }

    /// The smaller of two values, per component.
    ///
    /// # Errors
    ///
    /// A value from another program, or operand types the operation does not accept.
    pub fn min(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Min(left.expression, right.expression), &[left, right])
    }

    /// The larger of two values, per component.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn max(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Max(left.expression, right.expression), &[left, right])
    }

    /// `value` clamped to `low..=high`, per component.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn clamp(&mut self, value: Value, low: Value, high: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Clamp(value.expression, low.expression, high.expression),
            &[value, low, high],
        )
    }

    /// The dot product of two vectors of the same size.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn dot(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Dot(left.expression, right.expression), &[left, right])
    }

    /// The cross product of two `Vec3f`.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn cross(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Cross(left.expression, right.expression), &[left, right])
    }

    /// A vector scaled to length 1.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn normalize(&mut self, value: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Normalize(value.expression), &[value])
    }

    /// A vector's length, or a float's absolute value.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn length(&mut self, value: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Length(value.expression), &[value])
    }

    /// The greatest integer no larger than a float or each component of a float vector.
    ///
    /// # Errors
    ///
    /// A value from another program, or an operand that is not a `Float` or float vector.
    pub fn floor(&mut self, value: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Floor(value.expression), &[value])
    }

    /// `start + (end - start) * amount`, per component.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn mix(&mut self, start: Value, end: Value, amount: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Mix(start.expression, end.expression, amount.expression),
            &[start, end, amount],
        )
    }

    /// A texture read, to `Vec4f`. Sampling in the vertex stage is rejected by
    /// [`ProgramBuilder::finish`], which knows the stages.
    ///
    /// # Errors
    ///
    /// A value from another program, or operands that are not `(Texture2D, Sampler, Vec2f)`.
    pub fn sample(&mut self, texture: Value, sampler: Value, coordinates: Value) -> Result<Value, ValidationError> {
        let expression = Expression::Sample {
            texture: texture.expression,
            sampler: sampler.expression,
            coordinates: coordinates.expression,
        };
        self.operation(expression, &[texture, sampler, coordinates])
    }

    /// A `target` vector (`Vec2f`, `Vec3f`, or `Vec4f`) from floats and smaller vectors whose
    /// sizes add up to its, or from one `Float` repeated.
    ///
    /// # Errors
    ///
    /// A value from another program, a target that is not a float vector, or components that do
    /// not add up to it.
    pub fn construct(&mut self, target: Type, components: &[Value]) -> Result<Value, ValidationError> {
        for component in components {
            self.own(*component)?;
        }
        let ids: Vec<ExpressionId> = components.iter().map(|component| component.expression).collect();
        let Some(expression) = Expression::construct(target, &ids) else {
            return Err(self.error(ValidationErrorKind::InvalidOperation {
                expression: self.next(),
                operation: target.name(),
                operands: components.iter().map(|component| component.ty).collect(),
                expected: validate::construct_expected(target).unwrap_or(validate::CONSTRUCT_TARGET),
            }));
        };
        self.operation(expression, components)
    }

    /// Components read from a vector: `.w`, `.xyz`, `.zyx`.
    ///
    /// # Errors
    ///
    /// A value from another program, a non-vector, or a component the vector does not have.
    pub fn swizzle(&mut self, vector: Value, swizzle: Swizzle) -> Result<Value, ValidationError> {
        let expression = Expression::Swizzle {
            vector: vector.expression,
            swizzle,
        };
        self.operation(expression, &[vector])
    }

    /// Sets the vertex stage's clip-space position. [`ProgramBuilder::finish`] checks that it is
    /// a `Vec4f`.
    ///
    /// # Errors
    ///
    /// A value from another program.
    pub fn set_position(&mut self, value: Value) -> Result<(), ValidationError> {
        self.own(value)?;
        self.builder.program.set_position(value.expression);
        Ok(())
    }

    /// Adds a fragment output at the next color location and returns the location.
    /// [`ProgramBuilder::finish`] checks that it is a `Vec4f`.
    ///
    /// # Errors
    ///
    /// A value from another program.
    pub fn output(&mut self, value: Value) -> Result<u32, ValidationError> {
        self.own(value)?;
        Ok(self.builder.program.push_fragment_output(value.expression))
    }

    /// The id the next pushed expression gets.
    fn next(&self) -> ExpressionId {
        ExpressionId::at(self.builder.program.expressions.len())
    }

    fn error(&self, kind: ValidationErrorKind) -> ValidationError {
        ValidationError {
            kind,
            origin: self.origin.clone(),
        }
    }

    fn own(&self, value: Value) -> Result<(), ValidationError> {
        if value.program != self.builder.token {
            return Err(self.error(ValidationErrorKind::ForeignExpression {
                expression: value.expression,
            }));
        }
        Ok(())
    }

    fn check_declaration(
        &self,
        kind: DeclarationKind,
        earlier: &[Declaration],
        name: &str,
        ty: Type,
    ) -> Result<(), ValidationError> {
        let first = validate::declaration_errors(kind, earlier, name, ty).into_iter().next();
        first.map_or(Ok(()), |kind| Err(self.error(kind)))
    }

    /// Types a non-leaf `expression` over `operands`, then pushes it.
    fn operation(&mut self, expression: Expression, operands: &[Value]) -> Result<Value, ValidationError> {
        let mut types = [Type::Bool; 4];
        for (slot, operand) in operands.iter().enumerate() {
            self.own(*operand)?;
            types[slot] = operand.ty;
        }
        let id = self.next();
        let ty = validate::operation_type(id, &expression, &types[..operands.len()]).map_err(|kind| self.error(kind))?;
        validate::constant_out_of_range(&self.builder.program.expressions, id, &expression)
            .map_err(|kind| self.error(kind))?;
        Ok(self.push(expression, ty))
    }

    fn push(&mut self, expression: Expression, ty: Type) -> Value {
        let program = &mut self.builder.program;
        let id = match &self.origin {
            Some(origin) => program.push_at(expression, origin.clone()),
            None => program.push(expression),
        };
        Value {
            program: self.builder.token,
            expression: id,
            ty,
        }
    }
}
