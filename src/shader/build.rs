//! A typed symbolic builder over the IR, for a frontend that constructs a program one expression
//! at a time and wants each mistake reported where the author wrote it.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::shader::expression::{
    BinaryOperator, Builtin, Expression, ExpressionId, Function, Literal, Place, PlaceRoot, SampleLevel, SourceOrigin, Step,
    Swizzle, TextureQuery, UnaryOperator,
};
use crate::shader::flow::{Local, LocalId, LocalKind, Stage, Statement};
use crate::shader::interface::{
    Access, Attribute, AttributeId, DataType, Element, Field, Frequency, Rate, Resource, ResourceId, ResourceKind,
    StorageBuffer, StorageId, StructDecl, StructId, Uniform, UniformId,
};
use crate::shader::program::{Declaration, Program};
use crate::shader::types::Type;
use crate::shader::validate::{self, DeclarationKind, PlaceProblem, ValidationError, ValidationErrorKind};

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
    /// The stage being built, once one is entered.
    stage: Option<Stage>,
    /// The `if`s and loops open, innermost last.
    open: Vec<Open>,
}

/// An `if` or loop under construction.
#[derive(Debug)]
enum Open {
    If {
        condition: ExpressionId,
        accept: Vec<Statement>,
        /// `Some` once the else began.
        reject: Option<Vec<Statement>>,
        origin: Option<SourceOrigin>,
    },
    Loop {
        index: LocalId,
        count: ExpressionId,
        body: Vec<Statement>,
        origin: Option<SourceOrigin>,
    },
}

impl Default for ProgramBuilder {
    fn default() -> ProgramBuilder {
        ProgramBuilder::new()
    }
}

impl ProgramBuilder {
    /// A builder for an empty program of a vertex and a fragment stage.
    #[must_use]
    pub fn new() -> ProgramBuilder {
        ProgramBuilder::of(Program::new())
    }

    /// A builder for an empty compute program, run in workgroups of `workgroup_size`
    /// invocations.
    #[must_use]
    pub fn compute(workgroup_size: [u32; 3]) -> ProgramBuilder {
        ProgramBuilder::of(Program::compute(workgroup_size))
    }

    fn of(program: Program) -> ProgramBuilder {
        ProgramBuilder {
            token: NEXT_PROGRAM.fetch_add(1, Ordering::Relaxed),
            program,
            stage: None,
            open: Vec::new(),
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
        if !self.open.is_empty() {
            return Err(vec![ValidationError {
                kind: ValidationErrorKind::Unbalanced {
                    problem: "a shader.when or shader.loop is still open",
                },
                origin: None,
            }]);
        }
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

    /// Declares a storage buffer `name` that `frequency` binds: as many `element`s as the buffer
    /// bound holds, which the stages read, or read and write.
    ///
    /// # Errors
    ///
    /// An invalid name, a name the frequency has already, or an element no buffer holds.
    pub fn storage(
        &mut self,
        frequency: Frequency,
        name: &str,
        element: Element,
        access: Access,
    ) -> Result<StorageId, ValidationError> {
        let program = &self.builder.program;
        let kind = DeclarationKind::Storage(frequency);
        let names = binding_names(program, frequency, None);
        let mut problems = validate::binding_errors(kind, names, name);
        problems.extend(validate::data_type_errors(kind, name, element.into(), &program.structs));
        if let Some(problem) = problems.into_iter().next() {
            return Err(self.error(problem));
        }
        let mut buffer = StorageBuffer::new(frequency, name, element, access);
        buffer.origin.clone_from(&self.origin);
        Ok(self.builder.program.storage_buffer(buffer))
    }

    /// How many elements the storage buffer bound holds, a `UInt`.
    ///
    /// # Errors
    ///
    /// A storage buffer this program doesn't have.
    pub fn storage_length(&mut self, storage: StorageId) -> Result<Value, ValidationError> {
        if self.builder.program.storage.get(storage.index()).is_none() {
            return Err(self.error(ValidationErrorKind::InvalidPlace {
                expression: self.next(),
                problem: PlaceProblem::UndefinedStorage { storage },
            }));
        }
        Ok(self.push(Expression::StorageLength(storage), Type::UInt))
    }

    /// Writes `value` at `place`, in a `ReadWrite` storage buffer, from here on. `indices` are the
    /// values of its index steps, in order, as [`load`](Site::load) takes them.
    ///
    /// # Errors
    ///
    /// Values from another program, no stage entered, the vertex stage, a place outside a
    /// `ReadWrite` storage buffer or that doesn't lead to a value, or a value of another type.
    pub fn write(&mut self, place: Place, value: Value, indices: &[Value]) -> Result<(), ValidationError> {
        self.own(value)?;
        for index in indices {
            self.own(*index)?;
        }
        let stage = self.stage()?;
        let program = &self.builder.program;
        let target = match place.root {
            PlaceRoot::Uniform(_) => Some(ValidationErrorKind::WriteToUniform),
            PlaceRoot::Storage(storage) => match program.storage.get(storage.index()) {
                Some(buffer) if buffer.access == Access::Read => Some(ValidationErrorKind::WriteToReadOnly {
                    name: buffer.name.clone(),
                }),
                Some(buffer) if stage == Stage::Vertex => Some(ValidationErrorKind::StorageInVertex {
                    name: buffer.name.clone(),
                }),
                _ => None,
            },
        };
        if let Some(kind) = target {
            return Err(self.error(kind));
        }
        let index_type = |id: ExpressionId| {
            let index = indices.iter().find(|index| index.expression == id);
            index.map_or(Type::Bool, |index| index.ty)
        };
        let ty = validate::place_type(program, &place, index_type)
            .map_err(|problem| self.error(ValidationErrorKind::InvalidWrite { problem }))?;
        if ty != value.ty {
            return Err(self.error(ValidationErrorKind::WriteType {
                place: ty,
                value: value.ty,
            }));
        }
        self.statement(Statement::Write {
            place,
            value: value.expression,
            origin: self.origin.clone(),
        });
        Ok(())
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

    /// A built-in value, read at this site. Reading one in the stage that doesn't have it is
    /// reported by [`ProgramBuilder::finish`], which knows the stages.
    pub fn builtin(&mut self, builtin: Builtin) -> Value {
        self.push(Expression::Builtin(builtin), builtin.ty())
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

    /// `operator` on `value`.
    ///
    /// # Errors
    ///
    /// A value from another program, or a type the operator doesn't take.
    pub fn unary(&mut self, operator: UnaryOperator, value: Value) -> Result<Value, ValidationError> {
        self.operation(Expression::Unary(operator, value.expression), &[value])
    }

    /// `operator` between `left` and `right`.
    ///
    /// # Errors
    ///
    /// A value from another program, operand types the operator doesn't take, or constant
    /// arithmetic WGSL would reject.
    pub fn binary(&mut self, operator: BinaryOperator, left: Value, right: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Binary(operator, left.expression, right.expression),
            &[left, right],
        )
    }

    /// `accept` where `condition` holds, else `reject`.
    ///
    /// # Errors
    ///
    /// A value from another program, a condition that isn't a `Bool`, or choices that aren't one
    /// scalar or vector type.
    pub fn select(&mut self, condition: Value, accept: Value, reject: Value) -> Result<Value, ValidationError> {
        let expression = Expression::Select {
            condition: condition.expression,
            accept: accept.expression,
            reject: reject.expression,
        };
        self.operation(expression, &[condition, accept, reject])
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

    /// A float texture read, to `Vec4f`, at `coordinates` and, in an array texture, in `layer`,
    /// at the mip level the fragment stage's derivatives pick. Sampling in the vertex stage is
    /// rejected by [`ProgramBuilder::finish`], which knows the stages.
    ///
    /// # Errors
    ///
    /// Values from another program, or a texture, sampler, coordinates, and layer that don't
    /// fit: a float texture, a `Sampler`, the coordinates [`ResourceKind::coordinates`] gives,
    /// and a layer for an array texture only.
    pub fn sample(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
    ) -> Result<Value, ValidationError> {
        self.sample_at(texture, sampler, coordinates, layer, SampleLevel::Implicit, None)
    }

    /// [`sample`](Site::sample) at the level the derivatives pick, offset by a `Float` `bias`.
    /// Fragment stage only.
    ///
    /// # Errors
    ///
    /// As [`sample`](Site::sample), and a bias that isn't a `Float`.
    pub fn sample_bias(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
        bias: Value,
    ) -> Result<Value, ValidationError> {
        self.sample_at(
            texture,
            sampler,
            coordinates,
            layer,
            SampleLevel::Bias(bias.expression),
            Some(bias),
        )
    }

    /// [`sample`](Site::sample) at the `Float` mip `level` given, 0 the largest. Any stage.
    ///
    /// # Errors
    ///
    /// As [`sample`](Site::sample), and a level that isn't a `Float`.
    pub fn sample_level(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
        level: Value,
    ) -> Result<Value, ValidationError> {
        self.sample_at(
            texture,
            sampler,
            coordinates,
            layer,
            SampleLevel::Explicit(level.expression),
            Some(level),
        )
    }

    fn sample_at(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
        level: SampleLevel,
        level_value: Option<Value>,
    ) -> Result<Value, ValidationError> {
        let expression = Expression::Sample {
            texture,
            sampler,
            coordinates: coordinates.expression,
            layer: layer.map(|layer| layer.expression),
            level,
        };
        let values: Vec<Value> = [Some(coordinates), layer, level_value].into_iter().flatten().collect();
        self.texture_operation(expression, &values)
    }

    /// A depth texture's depths at `coordinates` and, in an array texture, in `layer`, compared
    /// with the `Float` `reference` through a comparison sampler and filtered, to a `Float` from
    /// 0 (every texel read fails) to 1 (every one passes), at the level the fragment stage's
    /// derivatives pick. Fragment stage only.
    ///
    /// # Errors
    ///
    /// Values from another program, or a texture, sampler, coordinates, layer, and reference
    /// that don't fit: a depth texture, a `ComparisonSampler`, its coordinates, a layer for an
    /// array texture only, and a `Float`.
    pub fn sample_compare(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
        reference: Value,
    ) -> Result<Value, ValidationError> {
        self.compare_at(texture, sampler, coordinates, layer, reference, false)
    }

    /// [`sample_compare`](Site::sample_compare) in mip level 0, in any stage.
    ///
    /// # Errors
    ///
    /// As [`sample_compare`](Site::sample_compare).
    pub fn sample_compare_level(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
        reference: Value,
    ) -> Result<Value, ValidationError> {
        self.compare_at(texture, sampler, coordinates, layer, reference, true)
    }

    fn compare_at(
        &mut self,
        texture: ResourceId,
        sampler: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
        reference: Value,
        level_zero: bool,
    ) -> Result<Value, ValidationError> {
        let expression = Expression::SampleCompare {
            texture,
            sampler,
            coordinates: coordinates.expression,
            layer: layer.map(|layer| layer.expression),
            reference: reference.expression,
            level_zero,
        };
        let values: Vec<Value> = [Some(coordinates), layer, Some(reference)].into_iter().flatten().collect();
        self.texture_operation(expression, &values)
    }

    /// One texel of a texture that isn't a cube, read without a sampler: at integer
    /// `coordinates`, in `layer` of an array texture, in the `Int` or `UInt` mip `level`. It reads
    /// as a `Vec4f`, or as a `Vec4i` or `Vec4u` from an integer texture.
    ///
    /// # Errors
    ///
    /// Values from another program, a cube, or coordinates, a layer, or a level that don't fit.
    pub fn load_texel(
        &mut self,
        texture: ResourceId,
        coordinates: Value,
        layer: Option<Value>,
        level: Value,
    ) -> Result<Value, ValidationError> {
        let expression = Expression::TextureLoad {
            texture,
            coordinates: coordinates.expression,
            layer: layer.map(|layer| layer.expression),
            level: level.expression,
        };
        let values: Vec<Value> = [Some(coordinates), layer, Some(level)].into_iter().flatten().collect();
        self.texture_operation(expression, &values)
    }

    /// A texture's size in texels, a `Vec2u` (a `Vec3u` for a 3D texture), of the `Int` or
    /// `UInt` mip `level` given, or of the largest.
    ///
    /// # Errors
    ///
    /// A level from another program or not an integer, or a resource that isn't a texture.
    pub fn dimensions(&mut self, texture: ResourceId, level: Option<Value>) -> Result<Value, ValidationError> {
        let query = TextureQuery::Dimensions(level.map(|level| level.expression));
        let values: Vec<Value> = level.into_iter().collect();
        self.texture_operation(Expression::TextureQuery { texture, query }, &values)
    }

    /// How many mip levels a texture has, a `UInt`.
    ///
    /// # Errors
    ///
    /// A resource that isn't a texture.
    pub fn levels(&mut self, texture: ResourceId) -> Result<Value, ValidationError> {
        let query = TextureQuery::Levels;
        self.texture_operation(Expression::TextureQuery { texture, query }, &[])
    }

    /// How many layers an array texture has, a `UInt`.
    ///
    /// # Errors
    ///
    /// A resource that isn't an array texture.
    pub fn layers(&mut self, texture: ResourceId) -> Result<Value, ValidationError> {
        let query = TextureQuery::Layers;
        self.texture_operation(Expression::TextureQuery { texture, query }, &[])
    }

    /// A texture read or query of `values`, its operands in order, typed against the program's
    /// resources.
    fn texture_operation(&mut self, expression: Expression, values: &[Value]) -> Result<Value, ValidationError> {
        for value in values {
            self.own(*value)?;
        }
        let id = self.next();
        let types: Vec<Type> = values.iter().map(|value| value.ty).collect();
        let ty = validate::texture_type(&self.builder.program, id, &expression, &types).map_err(|kind| self.error(kind))?;
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

    /// `value` as `target`, a scalar or vector type of the same size: `Float(index)`.
    ///
    /// # Errors
    ///
    /// A value from another program, or a matrix, or a value whose size isn't the target's.
    pub fn convert(&mut self, target: Type, value: Value) -> Result<Value, ValidationError> {
        self.operation(
            Expression::Convert {
                target,
                value: value.expression,
            },
            &[value],
        )
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
    /// Builds `stage`'s body from here on: every operation is computed where it is created, in
    /// whatever `if` or loop is open. Before the first `enter`, operations are placed nowhere and
    /// computed at the top of whichever stage reads them.
    ///
    /// # Errors
    ///
    /// An `if` or loop still open.
    pub fn enter(&mut self, stage: Stage) -> Result<(), ValidationError> {
        if !self.builder.open.is_empty() {
            return Err(self.unbalanced("a shader.when or shader.loop is still open"));
        }
        self.builder.stage = Some(stage);
        Ok(())
    }

    /// Declares a var of the current stage holding `initial`, and its type.
    ///
    /// # Errors
    ///
    /// A value from another program, or no stage entered.
    pub fn var(&mut self, initial: Value) -> Result<LocalId, ValidationError> {
        self.own(initial)?;
        let stage = self.stage()?;
        let local = self.builder.program.local(Local {
            ty: initial.ty,
            stage,
            kind: LocalKind::Var,
            origin: self.origin.clone(),
        });
        self.statement(Statement::Store {
            local,
            value: initial.expression,
            origin: self.origin.clone(),
        });
        Ok(local)
    }

    /// A local's value as of here.
    ///
    /// # Errors
    ///
    /// No stage entered, or a local this program doesn't have, or one of another stage.
    pub fn read_local(&mut self, local: LocalId) -> Result<Value, ValidationError> {
        let ty = self.local_of(local, None)?;
        Ok(self.push(Expression::Local(local), ty))
    }

    /// Stores `value` into a var, from here on.
    ///
    /// # Errors
    ///
    /// A value from another program, no stage entered, a local that isn't a var of this stage, or
    /// a value of another type.
    pub fn store(&mut self, local: LocalId, value: Value) -> Result<(), ValidationError> {
        self.own(value)?;
        let declared = self.local_of(local, Some(LocalKind::Var))?;
        if declared != value.ty {
            return Err(self.error(ValidationErrorKind::StoreType {
                declared,
                stored: value.ty,
            }));
        }
        self.statement(Statement::Store {
            local,
            value: value.expression,
            origin: self.origin.clone(),
        });
        Ok(())
    }

    /// Opens an `if` on a `Bool`: what follows runs where it holds, until
    /// [`begin_else`](Site::begin_else) or [`end_if`](Site::end_if).
    ///
    /// # Errors
    ///
    /// A value from another program, no stage entered, or a condition that isn't a `Bool`.
    pub fn begin_if(&mut self, condition: Value) -> Result<(), ValidationError> {
        self.own(condition)?;
        self.stage()?;
        if condition.ty != Type::Bool {
            return Err(self.error(ValidationErrorKind::ConditionType { found: condition.ty }));
        }
        self.builder.open.push(Open::If {
            condition: condition.expression,
            accept: Vec::new(),
            reject: None,
            origin: self.origin.clone(),
        });
        Ok(())
    }

    /// Switches the open `if` to what runs where its condition doesn't hold.
    ///
    /// # Errors
    ///
    /// No `if` open, or one already switched.
    pub fn begin_else(&mut self) -> Result<(), ValidationError> {
        match self.builder.open.last_mut() {
            Some(Open::If {
                reject: reject @ None, ..
            }) => {
                *reject = Some(Vec::new());
                Ok(())
            }
            _ => Err(self.unbalanced("an else with no shader.when open")),
        }
    }

    /// Closes the open `if`.
    ///
    /// # Errors
    ///
    /// No `if` open.
    pub fn end_if(&mut self) -> Result<(), ValidationError> {
        let Some(Open::If {
            condition,
            accept,
            reject,
            origin,
        }) = self.builder.open.pop_if(|open| matches!(open, Open::If { .. }))
        else {
            return Err(self.unbalanced("an end of a shader.when that isn't open"));
        };
        self.statement(Statement::If {
            condition,
            accept,
            reject: reject.unwrap_or_default(),
            origin,
        });
        Ok(())
    }

    /// Opens a loop that runs what follows `count` times, until [`end_loop`](Site::end_loop),
    /// and returns its index, counting from 0, of the count's type.
    ///
    /// # Errors
    ///
    /// A value from another program, no stage entered, or a count that isn't an `Int` or `UInt`.
    pub fn begin_loop(&mut self, count: Value) -> Result<Value, ValidationError> {
        self.own(count)?;
        let stage = self.stage()?;
        if !(count.ty.is_integer() && count.ty.is_scalar()) {
            return Err(self.error(ValidationErrorKind::LoopCountType {
                count: count.ty,
                index: count.ty,
            }));
        }
        let index = self.builder.program.local(Local {
            ty: count.ty,
            stage,
            kind: LocalKind::LoopIndex,
            origin: self.origin.clone(),
        });
        self.builder.open.push(Open::Loop {
            index,
            count: count.expression,
            body: Vec::new(),
            origin: self.origin.clone(),
        });
        Ok(self.push(Expression::Local(index), count.ty))
    }

    /// Closes the open loop.
    ///
    /// # Errors
    ///
    /// No loop open.
    pub fn end_loop(&mut self) -> Result<(), ValidationError> {
        let Some(Open::Loop {
            index,
            count,
            body,
            origin,
        }) = self.builder.open.pop_if(|open| matches!(open, Open::Loop { .. }))
        else {
            return Err(self.unbalanced("an end of a shader.loop that isn't open"));
        };
        self.statement(Statement::Loop {
            index,
            count,
            body,
            origin,
        });
        Ok(())
    }

    /// Throws the fragment away here: it writes no output.
    ///
    /// # Errors
    ///
    /// No stage entered, or a stage other than the fragment.
    pub fn discard(&mut self) -> Result<(), ValidationError> {
        let stage = self.stage()?;
        if stage != Stage::Fragment {
            return Err(self.error(ValidationErrorKind::DiscardOutsideFragment { stage }));
        }
        self.statement(Statement::Discard {
            origin: self.origin.clone(),
        });
        Ok(())
    }

    /// The stage being built, for a statement that needs one.
    fn stage(&self) -> Result<Stage, ValidationError> {
        self.builder
            .stage
            .ok_or_else(|| self.unbalanced("a statement outside the vertex and fragment functions"))
    }

    /// A local's type, if it is one of the current stage, and of `kind` if one is asked for.
    fn local_of(&self, local: LocalId, kind: Option<LocalKind>) -> Result<Type, ValidationError> {
        let stage = self.stage()?;
        let Some(declared) = self.builder.program.locals.get(local.index()) else {
            return Err(self.error(ValidationErrorKind::UndefinedLocal { local }));
        };
        if declared.stage != stage {
            return Err(self.error(ValidationErrorKind::LocalInOtherStage {
                kind: declared.kind,
                declared: declared.stage,
                used: stage,
            }));
        }
        if let Some(expected) = kind
            && declared.kind != expected
        {
            return Err(self.error(ValidationErrorKind::LocalKindMismatch { expected }));
        }
        Ok(declared.ty)
    }

    /// Appends `statement` to the open block: the innermost `if` or loop, or the stage's body.
    fn statement(&mut self, statement: Statement) {
        let builder = &mut *self.builder;
        let block = match builder.open.last_mut() {
            Some(Open::If {
                reject: Some(reject), ..
            }) => reject,
            Some(Open::If { accept, .. }) => accept,
            Some(Open::Loop { body, .. }) => body,
            None => match builder.stage {
                Some(stage) => builder.program.body_mut(stage),
                None => return,
            },
        };
        block.push(statement);
    }

    fn unbalanced(&self, problem: &'static str) -> ValidationError {
        self.error(ValidationErrorKind::Unbalanced { problem })
    }

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

    /// A call of `function` with `arguments`.
    ///
    /// # Errors
    ///
    /// A value from another program, the wrong number of arguments, or argument types the function
    /// doesn't take.
    pub fn call(&mut self, function: Function, arguments: &[Value]) -> Result<Value, ValidationError> {
        let ids: Vec<ExpressionId> = arguments.iter().map(|argument| argument.expression).collect();
        let Some(expression) = Expression::call(function, &ids) else {
            return Err(self.error(ValidationErrorKind::InvalidOperation {
                expression: self.next(),
                operation: function.name(),
                operands: arguments.iter().map(|argument| argument.ty).collect(),
                expected: "at most four arguments",
            }));
        };
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
        // A leaf is written where it is read; anything else is computed where it was made.
        if !expression.is_leaf() {
            self.statement(Statement::Emit(id));
        }
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
    let storage = program
        .storage
        .iter()
        .filter(move |buffer| buffer.frequency == frequency)
        .map(|buffer| buffer.name.as_str());
    uniforms.chain(resources).chain(storage)
}
