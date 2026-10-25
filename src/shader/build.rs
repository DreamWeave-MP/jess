//! A typed symbolic builder over the IR, for a frontend that constructs a program one expression
//! at a time and wants each mistake reported where the author wrote it.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::shader::expression::{
    BinaryOperator, Expression, ExpressionId, Function, Literal, Place, SourceOrigin, Step, Swizzle,
};
use crate::shader::interface::{
    Attribute, AttributeId, DataType, Field, Frequency, Rate, Resource, ResourceId, ResourceKind, StructDecl, StructId,
    Uniform, UniformId,
};
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
    /// The program so far, for a frontend that looks up what it has declared.
    #[must_use]
    pub fn program(&self) -> &Program {
        &self.builder.program
    }

    /// Declares a struct `name` of `fields`, which may name only the structs declared before it.
    ///
    /// # Errors
    ///
    /// The first problem with the declaration: an invalid name, no fields, or a field that is badly
    /// named, named twice, or of a type no buffer holds.
    pub fn structure(&mut self, name: &str, fields: Vec<Field>) -> Result<StructId, ValidationError> {
        let declaration = StructDecl {
            name: name.to_owned(),
            fields,
            origin: self.origin.clone(),
        };
        let problem = validate::struct_errors(&self.builder.program.structs, &declaration)
            .into_iter()
            .next();
        if let Some(kind) = problem {
            return Err(self.error(kind));
        }
        Ok(self.builder.program.structure(declaration))
    }

    /// Declares a member `name` of `ty` in the uniform block `frequency` binds. [`load`](Site::load)
    /// reads it.
    ///
    /// # Errors
    ///
    /// The first problem with the declaration: an invalid name, a name the frequency has already, or
    /// a type no buffer holds.
    pub fn uniform(&mut self, frequency: Frequency, name: &str, ty: DataType) -> Result<UniformId, ValidationError> {
        let program = &self.builder.program;
        let kind = DeclarationKind::Uniform(frequency);
        let mut problems = validate::binding_errors(kind, binding_names(program, frequency, None), name);
        problems.extend(validate::data_type_errors(kind, name, ty, &program.structs));
        if let Some(kind) = problems.into_iter().next() {
            return Err(self.error(kind));
        }
        let uniform = Uniform {
            name: name.to_owned(),
            frequency,
            ty,
            origin: self.origin.clone(),
        };
        Ok(self.builder.program.uniform(uniform))
    }

    /// Declares a texture or sampler `name` that `frequency` binds. A texture and a sampler may
    /// share a name; nothing else the frequency binds may.
    ///
    /// # Errors
    ///
    /// An invalid name, or a name the frequency has already.
    pub fn resource(&mut self, frequency: Frequency, name: &str, kind: ResourceKind) -> Result<ResourceId, ValidationError> {
        let program = &self.builder.program;
        let names = binding_names(program, frequency, Some(kind));
        let problem = validate::binding_errors(validate::resource_kind(frequency, kind), names, name)
            .into_iter()
            .next();
        if let Some(problem) = problem {
            return Err(self.error(problem));
        }
        let resource = Resource {
            name: name.to_owned(),
            frequency,
            kind,
            origin: self.origin.clone(),
        };
        Ok(self.builder.program.resource(resource))
    }

    /// The uniform value at `place`, read at this site. `indices` are the values of its index
    /// steps, in order: [`Place`] names them by expression, and each must be one of this program's.
    ///
    /// # Errors
    ///
    /// A value from another program, or a place that doesn't lead to a value: a member or field
    /// that isn't declared, an index that isn't an `Int` or `UInt` or is past a constant end, or a
    /// struct or array where a value should be.
    pub fn load(&mut self, place: Place, indices: &[Value]) -> Result<Value, ValidationError> {
        for index in indices {
            self.own(*index)?;
        }
        let index_type = |id: ExpressionId| {
            let index = indices.iter().find(|index| index.expression == id);
            index.map_or(Type::Bool, |index| index.ty)
        };
        let ty = validate::place_type(&self.builder.program, &place, index_type).map_err(|problem| {
            self.error(ValidationErrorKind::InvalidPlace {
                expression: self.next(),
                problem,
            })
        })?;
        debug_assert_eq!(
            place.steps().iter().filter(|step| matches!(step, Step::Index(_))).count(),
            indices.len()
        );
        Ok(self.push(Expression::Load(place), ty))
    }

    /// Declares a vertex attribute, per vertex or per instance. [`read_attribute`](Site::read_attribute)
    /// reads it, in the vertex stage only.
    ///
    /// # Errors
    ///
    /// The first problem with the declaration: an invalid name, a name already declared at its
    /// rate, or a type no vertex buffer holds.
    pub fn attribute(&mut self, rate: Rate, name: &str, ty: Type) -> Result<AttributeId, ValidationError> {
        let attribute = Attribute {
            name: name.to_owned(),
            rate,
            ty,
            origin: self.origin.clone(),
        };
        let problem = validate::attribute_errors(&self.builder.program.attributes, &attribute)
            .into_iter()
            .next();
        if let Some(kind) = problem {
            return Err(self.error(kind));
        }
        Ok(self.builder.program.attribute(attribute))
    }

    /// A declared attribute's value, read at this site. An id this program didn't declare is
    /// reported by [`ProgramBuilder::finish`].
    pub fn read_attribute(&mut self, attribute: AttributeId) -> Value {
        let ty = self
            .builder
            .program
            .attributes
            .get(attribute.index())
            .map_or(Type::Float, |declared| declared.ty);
        self.push(Expression::Attribute(attribute), ty)
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

    /// `left + right`.
    ///
    /// # Errors
    ///
    /// A value from another program, operand types the operation does not accept, or constant
    /// arithmetic WGSL would reject.
    pub fn add(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Binary(BinaryOperator::Add, left.expression, right.expression),
            &[left, right],
        )
    }

    /// `left - right`.
    ///
    /// # Errors
    ///
    /// As [`add`](Site::add).
    pub fn subtract(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Binary(BinaryOperator::Subtract, left.expression, right.expression),
            &[left, right],
        )
    }

    /// `left * right`, including matrix × vector and matrix × matrix.
    ///
    /// # Errors
    ///
    /// As [`add`](Site::add).
    pub fn multiply(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Binary(BinaryOperator::Multiply, left.expression, right.expression),
            &[left, right],
        )
    }

    /// `left / right`.
    ///
    /// # Errors
    ///
    /// As [`add`](Site::add).
    pub fn divide(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Binary(BinaryOperator::Divide, left.expression, right.expression),
            &[left, right],
        )
    }

    /// The smaller of two values, per component.
    ///
    /// # Errors
    ///
    /// A value from another program, or operand types the operation does not accept.
    pub fn min(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.call(Function::Min, &[left, right])
    }

    /// The larger of two values, per component.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn max(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.call(Function::Max, &[left, right])
    }

    /// `value` clamped to `low..=high`, per component.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn clamp(&mut self, value: Value, low: Value, high: Value) -> Result<Value, ValidationError> {
        self.call(Function::Clamp, &[value, low, high])
    }

    /// The dot product of two vectors of the same size.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn dot(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.call(Function::Dot, &[left, right])
    }

    /// The cross product of two `Vec3f`.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn cross(&mut self, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.call(Function::Cross, &[left, right])
    }

    /// A vector scaled to length 1.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn normalize(&mut self, value: Value) -> Result<Value, ValidationError> {
        self.call(Function::Normalize, &[value])
    }

    /// A vector's length, or a float's absolute value.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn length(&mut self, value: Value) -> Result<Value, ValidationError> {
        self.call(Function::Length, &[value])
    }

    /// The greatest integer no larger than a float or each component of a float vector.
    ///
    /// # Errors
    ///
    /// A value from another program, or an operand that is not a `Float` or float vector.
    pub fn floor(&mut self, value: Value) -> Result<Value, ValidationError> {
        self.call(Function::Floor, &[value])
    }

    /// `start + (end - start) * amount`, per component.
    ///
    /// # Errors
    ///
    /// As [`min`](Site::min).
    pub fn mix(&mut self, start: Value, end: Value, amount: Value) -> Result<Value, ValidationError> {
        self.call(Function::Mix, &[start, end, amount])
    }

    /// A texture read, to `Vec4f`. Sampling in the vertex stage is rejected by
    /// [`ProgramBuilder::finish`], which knows the stages.
    ///
    /// # Errors
    ///
    /// Coordinates from another program, or a texture, sampler, and coordinates that are not a
    /// `Texture2D`, a `Sampler`, and a `Vec2f`.
    pub fn sample(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
    ) -> Result<Value, ValidationError> {
        self.own(coordinates)?;
        let id = self.next();
        let program = &self.builder.program;
        let ty = validate::sample_type(program, id, texture, sampler, coordinates.ty).map_err(|kind| self.error(kind))?;
        let expression = Expression::Sample {
            texture,
            sampler,
            coordinates: coordinates.expression,
        };
        Ok(self.push(expression, ty))
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
        let earlier = earlier.iter().map(|declaration| (declaration.name.as_str(), declaration.ty));
        let first = validate::declaration_errors(kind, earlier, name, ty).into_iter().next();
        first.map_or(Ok(()), |kind| Err(self.error(kind)))
    }

    /// A call of `function` with `arguments`, typed and pushed.
    fn call(&mut self, function: Function, arguments: &[Value]) -> Result<Value, ValidationError> {
        let ids: Vec<ExpressionId> = arguments.iter().map(|argument| argument.expression).collect();
        let expression = Expression::call(function, &ids).expect("a function takes at most three arguments");
        self.operation(expression, arguments)
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

/// The names a `frequency` binds that a new uniform member (`kind` `None`) or resource of `kind`
/// may not take: every uniform member's, and every resource's of the same kind, or of any kind for
/// a uniform member.
fn binding_names(program: &Program, frequency: Frequency, kind: Option<ResourceKind>) -> impl Iterator<Item = &str> {
    let uniforms = program.uniforms_of(frequency).map(|(_, uniform)| uniform.name.as_str());
    let resources = program
        .resources
        .iter()
        .filter(move |resource| resource.frequency == frequency && kind.is_none_or(|kind| resource.kind == kind))
        .map(|resource| resource.name.as_str());
    uniforms.chain(resources)
}
