//! Type checking and stage checks, in Jess terms, before anything is lowered.

use std::fmt;

use crate::shader::expression::{
    BinaryOperator, Builtin, Expression, ExpressionId, Function, Literal, Place, PlaceRoot, SampleLevel, SourceOrigin, Step,
    Swizzle, TextureQuery, UnaryOperator, VaryingId,
};
use crate::shader::fold::{Arithmetic, evaluate};
use crate::shader::interface::{
    Attribute, AttributeId, DataType, Element, Frequency, Rate, ResourceId, ResourceKind, StructDecl, StructId,
};
use crate::shader::layout::ProgramLayout;
use crate::shader::program::{Declaration, Program};
use crate::shader::types::{Scalar, Type};

/// The most color outputs a fragment stage can have.
pub const MAX_FRAGMENT_OUTPUTS: usize = 8;
/// The most shader locations a program's attributes can take, both rates together.
pub const MAX_ATTRIBUTE_LOCATIONS: u32 = 16;

pub const VERTEX: u8 = 1;
pub const FRAGMENT: u8 = 2;

/// What kind of thing a declaration declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeclarationKind {
    /// A struct.
    Struct,
    /// A struct's field.
    Field,
    /// A vertex attribute of the rate.
    Attribute(Rate),
    /// A uniform member the frequency binds.
    Uniform(Frequency),
    /// A texture the frequency binds.
    Texture(Frequency),
    /// A sampler the frequency binds.
    Sampler(Frequency),
    /// A varying.
    Varying,
}

impl fmt::Display for DeclarationKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeclarationKind::Struct => formatter.write_str("struct"),
            DeclarationKind::Field => formatter.write_str("field"),
            DeclarationKind::Attribute(rate) => formatter.write_str(rate.name()),
            DeclarationKind::Uniform(frequency) => write!(formatter, "{frequency} uniform"),
            DeclarationKind::Texture(frequency) => write!(formatter, "{frequency} texture"),
            DeclarationKind::Sampler(frequency) => write!(formatter, "{frequency} sampler"),
            DeclarationKind::Varying => formatter.write_str("varying"),
        }
    }
}

/// One problem with a program, and where the author wrote it when that is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
    /// What is wrong.
    pub kind: ValidationErrorKind,
    /// The origin of the expression or declaration at fault, if recorded.
    pub origin: Option<SourceOrigin>,
}

/// What is wrong with a program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationErrorKind {
    /// A declaration's name is not ASCII letters, digits, and underscores starting with a letter or
    /// underscore.
    InvalidName {
        /// What the declaration declares.
        kind: DeclarationKind,
        /// The name.
        name: String,
    },
    /// Two declarations of the same kind would lower to the same backend name.
    DuplicateName {
        /// What the declarations declare.
        kind: DeclarationKind,
        /// The name.
        name: String,
    },
    /// A declaration has a type its kind cannot have, such as a `Bool` parameter or a `Mat4f`
    /// varying.
    UnsupportedDeclarationType {
        /// What the declaration declares.
        kind: DeclarationKind,
        /// The name.
        name: String,
        /// The type it was declared with.
        ty: Type,
    },
    /// An expression reads an expression that does not come before it in the arena.
    InvalidOperand {
        /// The reader.
        expression: ExpressionId,
        /// What it reads.
        operand: ExpressionId,
    },
    /// A stage output names an expression that is not in the program.
    UndefinedExpression {
        /// The missing expression.
        expression: ExpressionId,
    },
    /// A [`ProgramBuilder`](crate::shader::ProgramBuilder) was given a value another builder made.
    ForeignExpression {
        /// The expression, in the other program's arena.
        expression: ExpressionId,
    },
    /// A struct field or uniform member names a struct that isn't declared before the struct
    /// it belongs to, or at all.
    UndefinedStruct {
        /// What names it.
        kind: DeclarationKind,
        /// Its name.
        name: String,
        /// The struct named.
        structure: StructId,
    },
    /// A struct has no fields.
    EmptyStruct {
        /// The struct's name.
        name: String,
    },
    /// A struct field or uniform member is an array of no elements.
    EmptyArray {
        /// What it is.
        kind: DeclarationKind,
        /// Its name.
        name: String,
    },
    /// A uniform member is or holds an array whose elements are not a multiple of 16 bytes apart,
    /// which no uniform block can hold.
    UniformArrayStride {
        /// The array, from its frequency: `material.weights`, `pass.light.samples`.
        path: String,
        /// The bytes from one element to the next.
        stride: u32,
    },
    /// A load's place doesn't lead to a value.
    InvalidPlace {
        /// The load.
        expression: ExpressionId,
        /// What's wrong with it.
        problem: PlaceProblem,
    },
    /// An expression reads a texture or sampler that was never declared.
    UndefinedResource {
        /// The reader.
        expression: ExpressionId,
        /// The resource.
        resource: ResourceId,
    },
    /// A texture read or query is given what its texture doesn't take.
    InvalidTexture {
        /// The read or query.
        expression: ExpressionId,
        /// Its Jess name: `sample`, `load`, `dimensions`, ...
        operation: &'static str,
        /// What it was given: the resources' kinds, then the values' types.
        arguments: Vec<&'static str>,
        /// What it takes.
        expected: String,
    },
    /// An expression reads an attribute that was never declared.
    UndefinedAttribute {
        /// The reader.
        expression: ExpressionId,
        /// The attribute.
        attribute: AttributeId,
    },
    /// An expression reads, or the vertex stage writes, a varying that was never declared.
    UndefinedVarying {
        /// The reader, or the expression written.
        expression: ExpressionId,
        /// The varying.
        varying: VaryingId,
    },
    /// A float literal is NaN or infinite, which WGSL cannot write.
    NonFiniteLiteral {
        /// The literal.
        expression: ExpressionId,
    },
    /// An operation's operand types are not a combination it accepts: `dot(Vec3f, Vec4f)`.
    InvalidOperation {
        /// The operation.
        expression: ExpressionId,
        /// Its name: `+`, `dot`, `sample`, or a vector constructor's type.
        operation: &'static str,
        /// The operand types, in order.
        operands: Vec<Type>,
        /// What it accepts.
        expected: &'static str,
    },
    /// Arithmetic on two literals overflows an integer, divides an integer by zero, or is not
    /// finite. WGSL evaluates it while compiling and rejects it.
    ConstantOutOfRange {
        /// The arithmetic.
        expression: ExpressionId,
    },
    /// A shift by a constant of 32 or more, which WGSL rejects while compiling.
    ShiftTooFar {
        /// The shift.
        expression: ExpressionId,
        /// The amount.
        amount: u32,
    },
    /// A swizzle reads a component its vector does not have, or reads a non-vector.
    InvalidSwizzle {
        /// The swizzle.
        expression: ExpressionId,
        /// The type swizzled.
        vector: Type,
        /// The components.
        swizzle: Swizzle,
    },
    /// The fragment stage reads an attribute; it only sees varyings.
    AttributeInFragment {
        /// The reader.
        expression: ExpressionId,
        /// The attribute's name.
        attribute: String,
    },
    /// A stage reads a built-in value only the other stage has.
    BuiltinInStage {
        /// The reader.
        expression: ExpressionId,
        /// The value.
        builtin: Builtin,
    },
    /// The attributes take more than [`MAX_ATTRIBUTE_LOCATIONS`] locations.
    TooManyAttributeLocations {
        /// How many they take.
        count: u32,
    },
    /// The vertex stage reads a varying; it only writes them.
    VaryingInVertex {
        /// The reader.
        expression: ExpressionId,
        /// The varying's name.
        varying: String,
    },
    /// The vertex stage samples a texture at a level the fragment stage's derivatives pick.
    SampleInVertex {
        /// The sample.
        expression: ExpressionId,
        /// `sample` or `sampleBias`.
        operation: &'static str,
    },
    /// The vertex stage takes a derivative, which only the fragment stage has.
    DerivativeInVertex {
        /// The call.
        expression: ExpressionId,
        /// `dpdx`, `dpdy`, or `fwidth`.
        function: Function,
    },
    /// The vertex stage has no position.
    MissingPosition,
    /// The vertex position is not a `Vec4f`.
    PositionType {
        /// Its type.
        found: Type,
    },
    /// A fragment output is not a `Vec4f`.
    FragmentOutputType {
        /// The output's color location.
        location: u32,
        /// Its type.
        found: Type,
    },
    /// The fragment stage has more than [`MAX_FRAGMENT_OUTPUTS`] outputs.
    TooManyFragmentOutputs {
        /// How many it has.
        count: usize,
    },
    /// The fragment stage reads a varying the vertex stage never writes.
    VaryingNotWritten {
        /// The varying's name.
        varying: String,
    },
    /// The vertex stage writes a varying more than once.
    VaryingWrittenTwice {
        /// The varying's name.
        varying: String,
    },
    /// The vertex stage writes a varying a value of another type.
    VaryingType {
        /// The varying's name.
        varying: String,
        /// Its declared type.
        declared: Type,
        /// The type written.
        written: Type,
    },
}

impl fmt::Display for ValidationErrorKind {
    #[allow(clippy::too_many_lines)] // one arm per error
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationErrorKind::InvalidName { kind, name } => write!(
                formatter,
                "{kind} name {name:?} is invalid: expected ASCII letters, digits, and underscores, not starting with a digit"
            ),
            ValidationErrorKind::DuplicateName { kind, name } => {
                write!(formatter, "{kind} {name:?} is declared twice")
            }
            ValidationErrorKind::UnsupportedDeclarationType { kind, name, ty } => {
                let expected = match kind {
                    DeclarationKind::Attribute(_) => "Int, UInt, Float, a vector, or a matrix",
                    DeclarationKind::Varying => "Int, UInt, Float, or a vector",
                    _ => "Int, UInt, Float, a vector, a matrix, a struct, or an array of one of those",
                };
                write!(formatter, "{kind} {name:?} cannot be {ty}: expected {expected}")
            }
            ValidationErrorKind::InvalidOperand { expression, operand } => write!(
                formatter,
                "expression {} reads expression {}, which does not come before it",
                expression.index(),
                operand.index()
            ),
            ValidationErrorKind::UndefinedExpression { expression } => {
                write!(formatter, "expression {} is not in this program", expression.index())
            }
            ValidationErrorKind::ForeignExpression { expression } => {
                write!(
                    formatter,
                    "expression {} belongs to another shader program",
                    expression.index()
                )
            }
            ValidationErrorKind::UndefinedStruct { kind, name, structure } => write!(
                formatter,
                "{kind} {name:?} names struct {}, which is not declared before it",
                structure.index()
            ),
            ValidationErrorKind::EmptyStruct { name } => write!(formatter, "struct {name:?} has no fields"),
            ValidationErrorKind::EmptyArray { kind, name } => {
                write!(formatter, "{kind} {name:?} is an array of no elements")
            }
            ValidationErrorKind::UniformArrayStride { path, stride } => write!(
                formatter,
                "uniform {path} is an array of elements {stride} bytes apart: a uniform array's elements \
                 must be a multiple of 16 bytes apart, so give it 16-byte elements (a Vec4f, a struct), or \
                 read it from a storage buffer"
            ),
            ValidationErrorKind::InvalidPlace { problem, .. } => write!(formatter, "a load {problem}"),
            ValidationErrorKind::UndefinedResource { expression, resource } => write!(
                formatter,
                "expression {} reads resource {}, which is not declared",
                expression.index(),
                resource.index()
            ),
            ValidationErrorKind::InvalidTexture {
                operation,
                arguments,
                expected,
                ..
            } => write!(
                formatter,
                "{operation}({}) is invalid: expected {expected}",
                arguments.join(", ")
            ),
            ValidationErrorKind::UndefinedAttribute { expression, attribute } => write!(
                formatter,
                "expression {} reads attribute {}, which is not declared",
                expression.index(),
                attribute.index()
            ),
            ValidationErrorKind::UndefinedVarying { expression, varying } => write!(
                formatter,
                "expression {} uses varying {}, which is not declared",
                expression.index(),
                varying.index()
            ),
            ValidationErrorKind::NonFiniteLiteral { expression } => write!(
                formatter,
                "expression {} is a NaN or infinite literal, which WGSL cannot write",
                expression.index()
            ),
            ValidationErrorKind::InvalidOperation {
                operation,
                operands,
                expected,
                ..
            } => {
                if let [left, right] = operands.as_slice()
                    && matches!(*operation, "+" | "-" | "*" | "/" | "%")
                {
                    return write!(formatter, "{left} {operation} {right} is invalid: expected {expected}");
                }
                write!(formatter, "{operation}(")?;
                for (position, operand) in operands.iter().enumerate() {
                    if position > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{operand}")?;
                }
                write!(formatter, ") is invalid: expected {expected}")
            }
            ValidationErrorKind::ConstantOutOfRange { expression } => write!(
                formatter,
                "expression {} is constant arithmetic that overflows, divides an integer by zero, or is not finite",
                expression.index()
            ),
            ValidationErrorKind::ShiftTooFar { amount, .. } => {
                write!(formatter, "a shift by {amount} is too far: shift by less than 32")
            }
            ValidationErrorKind::InvalidSwizzle { vector, swizzle, .. } => {
                write!(formatter, "{vector}.{swizzle} is invalid: expected components a {vector} has")
            }
            ValidationErrorKind::AttributeInFragment { attribute, .. } => write!(
                formatter,
                "attribute {attribute:?} is read in the fragment stage: pass it through a varying"
            ),
            ValidationErrorKind::BuiltinInStage { builtin, .. } => {
                let (reader, owner) = if builtin.in_vertex() {
                    ("fragment", "vertex")
                } else {
                    ("vertex", "fragment")
                };
                write!(
                    formatter,
                    "builtin {} is read in the {reader} stage: only the {owner} stage has it",
                    builtin.name()
                )
            }
            ValidationErrorKind::TooManyAttributeLocations { count } => write!(
                formatter,
                "the attributes take {count} locations: at most {MAX_ATTRIBUTE_LOCATIONS}"
            ),
            ValidationErrorKind::VaryingInVertex { varying, .. } => write!(
                formatter,
                "varying {varying:?} is read in the vertex stage, which only writes varyings"
            ),
            ValidationErrorKind::SampleInVertex { operation, .. } => write!(
                formatter,
                "{operation} is used in the vertex stage, which has no derivatives to pick a mip level with: \
                 sample at a level with sampleLevel"
            ),
            ValidationErrorKind::DerivativeInVertex { function, .. } => write!(
                formatter,
                "{} is used in the vertex stage: derivatives are taken in the fragment stage",
                function.name()
            ),
            ValidationErrorKind::MissingPosition => formatter.write_str("the vertex stage has no position"),
            ValidationErrorKind::PositionType { found } => {
                write!(formatter, "the vertex position is {found}: expected Vec4f")
            }
            ValidationErrorKind::FragmentOutputType { location, found } => {
                write!(formatter, "fragment output {location} is {found}: expected Vec4f")
            }
            ValidationErrorKind::TooManyFragmentOutputs { count } => write!(
                formatter,
                "the fragment stage has {count} outputs: at most {MAX_FRAGMENT_OUTPUTS}"
            ),
            ValidationErrorKind::VaryingNotWritten { varying } => write!(
                formatter,
                "varying {varying:?} is read by the fragment stage but never written by the vertex stage"
            ),
            ValidationErrorKind::VaryingWrittenTwice { varying } => {
                write!(formatter, "varying {varying:?} is written twice")
            }
            ValidationErrorKind::VaryingType {
                varying,
                declared,
                written,
            } => {
                write!(formatter, "varying {varying:?} is declared {declared} but written {written}")
            }
        }
    }
}

/// Why a [`Place`] doesn't lead to a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceProblem {
    /// It takes no step: a uniform block itself is not a value.
    Empty,
    /// Its first step is not a member of the root's uniform block.
    UndefinedMember {
        /// The block's frequency.
        frequency: Frequency,
        /// The member's position, as the step gives it.
        member: u32,
    },
    /// It takes a field of something that is not a struct.
    NotAStruct,
    /// It takes a field the struct doesn't have.
    UndefinedField {
        /// The struct's name.
        structure: String,
        /// The field's position, as the step gives it.
        field: u32,
    },
    /// It indexes something that is not an array.
    NotAnArray,
    /// An index is not an `Int` or `UInt`.
    IndexType {
        /// The index's type.
        found: Type,
    },
    /// A constant index is past the array's end.
    IndexOutOfRange {
        /// The index.
        index: i64,
        /// How many elements the array has.
        length: u32,
    },
    /// It ends at a struct or an array.
    NotAValue,
}

impl fmt::Display for PlaceProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaceProblem::Empty => formatter.write_str("names a uniform block, not one of its members"),
            PlaceProblem::UndefinedMember { frequency, member } => {
                write!(
                    formatter,
                    "reads member {member} of the {frequency} uniforms, which is not declared"
                )
            }
            PlaceProblem::NotAStruct => formatter.write_str("takes a field of something that is not a struct"),
            PlaceProblem::UndefinedField { structure, field } => {
                write!(formatter, "reads field {field} of {structure}, which it doesn't have")
            }
            PlaceProblem::NotAnArray => formatter.write_str("indexes something that is not an array"),
            PlaceProblem::IndexType { found } => {
                write!(formatter, "indexes an array with a {found}: expected an Int or UInt")
            }
            PlaceProblem::IndexOutOfRange { index, length } => {
                write!(formatter, "reads element {index} of an array of {length}")
            }
            PlaceProblem::NotAValue => {
                formatter.write_str("ends at a struct or an array, not a value: read one of its fields or elements")
            }
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.origin {
            Some(origin) => write!(formatter, "{origin}: {}", self.kind),
            None => write!(formatter, "{}", self.kind),
        }
    }
}

impl std::error::Error for ValidationError {}

/// A validated program's facts: which stages reach each expression, and its type.
#[derive(Debug)]
pub struct Analysis {
    /// `VERTEX | FRAGMENT` bits per expression; 0 for expressions no stage reads.
    pub(crate) stages: Vec<u8>,
    /// Every reachable expression's type; `None` for unreachable ones.
    pub(crate) types: Vec<Option<Type>>,
}

impl Program {
    /// Checks the whole program, collecting every problem it finds.
    ///
    /// Only expressions a stage reads are checked: an unused, ill-typed expression is not an error.
    ///
    /// # Errors
    ///
    /// Every problem found, in declaration, then stage, then arena order.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        analyze(self).map(|_| ())
    }
}

pub fn analyze(program: &Program) -> Result<Analysis, Vec<ValidationError>> {
    let mut errors = Vec::new();
    check_declarations(program, &mut errors);
    let stages = mark_stages(program, &mut errors);
    let mut varyings_read = vec![false; program.varyings.len()];
    let types = infer_types(program, &stages, &mut varyings_read, &mut errors);
    check_outputs(program, &types, &varyings_read, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(Analysis { stages, types })
}

fn check_declarations(program: &Program, errors: &mut Vec<ValidationError>) {
    let mut report = |kinds: Vec<ValidationErrorKind>, origin: &Option<SourceOrigin>| {
        errors.extend(kinds.into_iter().map(|kind| ValidationError {
            kind,
            origin: origin.clone(),
        }));
    };
    for (index, declaration) in program.structs.iter().enumerate() {
        report(struct_errors(&program.structs[..index], declaration), &declaration.origin);
    }
    for (index, attribute) in program.attributes.iter().enumerate() {
        report(attribute_errors(&program.attributes[..index], attribute), &attribute.origin);
    }
    for (index, uniform) in program.uniforms.iter().enumerate() {
        let earlier = program.uniforms[..index]
            .iter()
            .filter(|earlier| earlier.frequency == uniform.frequency)
            .map(|earlier| earlier.name.as_str());
        let mut kinds = binding_errors(DeclarationKind::Uniform(uniform.frequency), earlier, &uniform.name);
        kinds.extend(data_type_errors(
            DeclarationKind::Uniform(uniform.frequency),
            &uniform.name,
            uniform.ty,
            &program.structs,
        ));
        report(kinds, &uniform.origin);
    }
    for (index, resource) in program.resources.iter().enumerate() {
        let kind = resource_kind(resource.frequency, resource.kind);
        // A texture and the sampler declared with it share a name; nothing else of a frequency
        // does.
        let earlier = program
            .uniforms_of(resource.frequency)
            .map(|(_, uniform)| uniform.name.as_str())
            .chain(
                program.resources[..index]
                    .iter()
                    .filter(|earlier| earlier.frequency == resource.frequency && earlier.kind == resource.kind)
                    .map(|earlier| earlier.name.as_str()),
            );
        report(binding_errors(kind, earlier, &resource.name), &resource.origin);
    }
    check_named(DeclarationKind::Varying, &program.varyings, errors);

    let layout = program.layout();
    for uniform in &program.uniforms {
        let path = format!("{}.{}", uniform.frequency, uniform.name);
        uniform_stride_errors(program, &layout, uniform.ty, &path, errors);
    }
    let count = layout
        .vertex_buffers
        .iter()
        .flat_map(|buffer| &buffer.attributes)
        .map(|layout| layout.locations)
        .sum();
    if count > MAX_ATTRIBUTE_LOCATIONS {
        errors.push(ValidationError {
            kind: ValidationErrorKind::TooManyAttributeLocations { count },
            origin: None,
        });
    }
}

/// Checks each of `declarations` against the ones before it.
fn check_named(kind: DeclarationKind, declarations: &[Declaration], errors: &mut Vec<ValidationError>) {
    for (index, declaration) in declarations.iter().enumerate() {
        let Declaration { name, ty, origin } = declaration;
        let earlier = declarations[..index]
            .iter()
            .map(|earlier| (earlier.name.as_str(), earlier.ty));
        for kind in declaration_errors(kind, earlier, name, *ty) {
            errors.push(ValidationError {
                kind,
                origin: origin.clone(),
            });
        }
    }
}

/// What is wrong with declaring `declaration` after the `earlier` structs, which are the only ones
/// its fields may name: an invalid name, no fields, or a field that is badly named, named twice, or
/// of a type no buffer holds. Empty when nothing is.
pub fn struct_errors(earlier: &[StructDecl], declaration: &StructDecl) -> Vec<ValidationErrorKind> {
    let mut errors = Vec::new();
    if !is_valid_name(&declaration.name) {
        errors.push(ValidationErrorKind::InvalidName {
            kind: DeclarationKind::Struct,
            name: declaration.name.clone(),
        });
    }
    if declaration.fields.is_empty() {
        errors.push(ValidationErrorKind::EmptyStruct {
            name: declaration.name.clone(),
        });
    }
    for (index, field) in declaration.fields.iter().enumerate() {
        let names = declaration.fields[..index].iter().map(|earlier| earlier.name.as_str());
        errors.extend(binding_errors(DeclarationKind::Field, names, &field.name));
        errors.extend(data_type_errors(DeclarationKind::Field, &field.name, field.ty, earlier));
    }
    errors
}

/// What is wrong with the name of a uniform member, resource, or field declared after the `earlier`
/// names it may not share: an invalid name, or one already taken.
pub fn binding_errors<'a>(
    kind: DeclarationKind,
    mut earlier: impl Iterator<Item = &'a str>,
    name: &str,
) -> Vec<ValidationErrorKind> {
    let mut errors = Vec::new();
    if !is_valid_name(name) {
        errors.push(ValidationErrorKind::InvalidName {
            kind,
            name: name.to_owned(),
        });
    }
    if earlier.any(|earlier| earlier == name) {
        errors.push(ValidationErrorKind::DuplicateName {
            kind,
            name: name.to_owned(),
        });
    }
    errors
}

/// What is wrong with a field or uniform member `name` of `ty`, which may name only the `structs`:
/// a `Bool`, a struct not among them, or an array of no elements.
pub fn data_type_errors(
    kind: DeclarationKind,
    name: &str,
    ty: DataType,
    structs: &[StructDecl],
) -> Vec<ValidationErrorKind> {
    let element = match ty {
        DataType::Value(ty) => Element::Value(ty),
        DataType::Struct(id) => Element::Struct(id),
        DataType::Array { element, length } => {
            if length == 0 {
                return vec![ValidationErrorKind::EmptyArray {
                    kind,
                    name: name.to_owned(),
                }];
            }
            element
        }
    };
    match element {
        Element::Value(Type::Bool) => vec![ValidationErrorKind::UnsupportedDeclarationType {
            kind,
            name: name.to_owned(),
            ty: Type::Bool,
        }],
        Element::Struct(structure) if structure.index() >= structs.len() => vec![ValidationErrorKind::UndefinedStruct {
            kind,
            name: name.to_owned(),
            structure,
        }],
        Element::Value(_) | Element::Struct(_) => Vec::new(),
    }
}

/// The declaration kind of a resource of `kind` that `frequency` binds.
pub fn resource_kind(frequency: Frequency, kind: ResourceKind) -> DeclarationKind {
    match kind {
        ResourceKind::Sampler => DeclarationKind::Sampler(frequency),
        _ => DeclarationKind::Texture(frequency),
    }
}

/// Reports every array in a uniform member of `ty`, at `path`, whose elements are not a multiple of
/// 16 bytes apart, looking through structs.
fn uniform_stride_errors(
    program: &Program,
    layout: &ProgramLayout,
    ty: DataType,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let structure = match ty {
        DataType::Value(_) => return,
        DataType::Struct(id) => Some(id),
        DataType::Array { element, .. } => {
            let stride = layout.element_stride(element);
            if !stride.is_multiple_of(16) {
                errors.push(ValidationError {
                    kind: ValidationErrorKind::UniformArrayStride {
                        path: path.to_owned(),
                        stride,
                    },
                    origin: None,
                });
                return;
            }
            match element {
                Element::Struct(id) => Some(id),
                Element::Value(_) => None,
            }
        }
    };
    let Some(declaration) = structure.and_then(|id| program.structs.get(id.index())) else {
        return;
    };
    for field in &declaration.fields {
        uniform_stride_errors(program, layout, field.ty, &format!("{path}.{}", field.name), errors);
    }
}

/// What is wrong with declaring `attribute` after `earlier` attributes: an invalid name, a name
/// already taken at its rate, or a type no vertex buffer holds. Empty when nothing is.
pub fn attribute_errors(earlier: &[Attribute], attribute: &Attribute) -> Vec<ValidationErrorKind> {
    let earlier = earlier
        .iter()
        .filter(|earlier| earlier.rate == attribute.rate)
        .map(|earlier| (earlier.name.as_str(), earlier.ty));
    declaration_errors(
        DeclarationKind::Attribute(attribute.rate),
        earlier,
        &attribute.name,
        attribute.ty,
    )
}

/// What is wrong with declaring `name` as `ty` after `earlier` declarations of the same `kind`:
/// an invalid name, a name already taken, or a type the kind cannot have. Empty when nothing is.
pub fn declaration_errors<'a>(
    kind: DeclarationKind,
    mut earlier: impl Iterator<Item = (&'a str, Type)>,
    name: &str,
    ty: Type,
) -> Vec<ValidationErrorKind> {
    let mut errors = Vec::new();
    if !is_valid_name(name) {
        errors.push(ValidationErrorKind::InvalidName {
            kind,
            name: name.to_owned(),
        });
    }
    if earlier.any(|(earlier, _)| earlier == name) {
        errors.push(ValidationErrorKind::DuplicateName {
            kind,
            name: name.to_owned(),
        });
    }
    let supported = match kind {
        DeclarationKind::Attribute(_) => ty.is_numeric() || ty.is_matrix(),
        DeclarationKind::Varying => ty.is_numeric(),
        _ => ty != Type::Bool,
    };
    if !supported {
        errors.push(ValidationErrorKind::UnsupportedDeclarationType {
            kind,
            name: name.to_owned(),
            ty,
        });
    }
    errors
}

fn is_valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == b'_') && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Marks which stages reach each expression, walking the arena backwards: operands always come
/// first, so one reverse pass visits every reader before what it reads.
fn mark_stages(program: &Program, errors: &mut Vec<ValidationError>) -> Vec<u8> {
    let count = program.expressions.len();
    let mut stages = vec![0u8; count];
    let mut mark_root = |expression: ExpressionId, stage: u8, errors: &mut Vec<ValidationError>| {
        if expression.index() >= count {
            errors.push(ValidationError {
                kind: ValidationErrorKind::UndefinedExpression { expression },
                origin: None,
            });
            return;
        }
        stages[expression.index()] |= stage;
    };

    match program.position {
        Some(position) => mark_root(position, VERTEX, errors),
        None => errors.push(ValidationError {
            kind: ValidationErrorKind::MissingPosition,
            origin: None,
        }),
    }
    let mut written = vec![false; program.varyings.len()];
    for &(varying, expression) in &program.varying_writes {
        mark_root(expression, VERTEX, errors);
        let Some(was_written) = written.get_mut(varying.index()) else {
            errors.push(ValidationError {
                kind: ValidationErrorKind::UndefinedVarying { expression, varying },
                origin: program.origin(expression).cloned(),
            });
            continue;
        };
        if *was_written {
            let declaration = &program.varyings[varying.index()];
            errors.push(ValidationError {
                kind: ValidationErrorKind::VaryingWrittenTwice {
                    varying: declaration.name.clone(),
                },
                origin: program.origin(expression).cloned(),
            });
        }
        *was_written = true;
    }
    if program.fragment_outputs.len() > MAX_FRAGMENT_OUTPUTS {
        errors.push(ValidationError {
            kind: ValidationErrorKind::TooManyFragmentOutputs {
                count: program.fragment_outputs.len(),
            },
            origin: None,
        });
    }
    for &output in &program.fragment_outputs {
        mark_root(output, FRAGMENT, errors);
    }

    for index in (0..count).rev() {
        let stage = stages[index];
        if stage == 0 {
            continue;
        }
        for &operand in program.expressions[index].operands().iter() {
            if operand.index() >= index {
                let expression = ExpressionId::at(index);
                errors.push(ValidationError {
                    kind: ValidationErrorKind::InvalidOperand { expression, operand },
                    origin: program.origin(expression).cloned(),
                });
                continue;
            }
            stages[operand.index()] |= stage;
        }
    }
    stages
}

fn infer_types(
    program: &Program,
    stages: &[u8],
    varyings_read: &mut [bool],
    errors: &mut Vec<ValidationError>,
) -> Vec<Option<Type>> {
    let mut types: Vec<Option<Type>> = vec![None; program.expressions.len()];
    for (index, expression) in program.expressions.iter().enumerate() {
        let stage = stages[index];
        if stage == 0 {
            continue;
        }
        let id = ExpressionId::at(index);
        let result = infer(program, id, expression, stage, &types, varyings_read);
        types[index] = match result {
            Ok(ty) => ty,
            Err(kind) => {
                errors.push(ValidationError {
                    kind,
                    origin: program.origin(id).cloned(),
                });
                None
            }
        };
    }
    types
}

/// What a vector constructor's target must be.
pub const CONSTRUCT_TARGET: &str = "a vector or matrix target";
const FLOAT: &str = "a Float or float vector";
const ARITHMETIC: &str = "two values of the same numeric type, or a vector and a scalar of its kind";
const ADD_SUBTRACT: &str =
    "two values of the same numeric type, a vector and a scalar of its kind, or two matrices of the same size";
const MULTIPLY: &str = "two values of the same numeric type, a vector and a scalar of its kind, a matrix and a Float, \
                        a matrix and a vector of its size, or two matrices of the same size";

/// The type of one expression, `Ok(None)` when an operand already failed (so one mistake is
/// reported once), or what is wrong with it.
fn infer(
    program: &Program,
    id: ExpressionId,
    expression: &Expression,
    stage: u8,
    types: &[Option<Type>],
    varyings_read: &mut [bool],
) -> Result<Option<Type>, ValidationErrorKind> {
    let operand_ids = expression.operands();
    let mut operand_types = [Type::Bool; 4];
    for (slot, operand) in operand_ids.iter().enumerate() {
        // An operand that does not come first was reported by `mark_stages`.
        if operand.index() >= id.index() {
            return Ok(None);
        }
        let Some(ty) = types[operand.index()] else {
            return Ok(None);
        };
        operand_types[slot] = ty;
    }
    let operands = &operand_types[..operand_ids.len()];

    let ty = match *expression {
        Expression::Literal(literal) => literal_type(id, literal)?,
        // Every index was typed: an untyped operand returned above.
        Expression::Load(place) => place_type(program, &place, |index| types[index.index()].unwrap_or(Type::Bool))
            .map_err(|problem| ValidationErrorKind::InvalidPlace { expression: id, problem })?,
        Expression::Attribute(attribute) => {
            let Some(declaration) = program.attributes.get(attribute.index()) else {
                return Err(ValidationErrorKind::UndefinedAttribute {
                    expression: id,
                    attribute,
                });
            };
            if stage & FRAGMENT != 0 {
                return Err(ValidationErrorKind::AttributeInFragment {
                    expression: id,
                    attribute: declaration.name.clone(),
                });
            }
            declaration.ty
        }
        Expression::Builtin(builtin) => {
            let other = if builtin.in_vertex() { FRAGMENT } else { VERTEX };
            if stage & other != 0 {
                return Err(ValidationErrorKind::BuiltinInStage { expression: id, builtin });
            }
            builtin.ty()
        }
        Expression::Varying(varying) => {
            let Some(declaration) = program.varyings.get(varying.index()) else {
                return Err(ValidationErrorKind::UndefinedVarying { expression: id, varying });
            };
            if stage & VERTEX != 0 {
                return Err(ValidationErrorKind::VaryingInVertex {
                    expression: id,
                    varying: declaration.name.clone(),
                });
            }
            varyings_read[varying.index()] = true;
            declaration.ty
        }
        Expression::Sample { level, .. } if level.needs_derivatives() && stage & VERTEX != 0 => {
            return Err(ValidationErrorKind::SampleInVertex {
                expression: id,
                operation: level.operation(),
            });
        }
        Expression::Call { function, .. } if function.needs_derivatives() && stage & VERTEX != 0 => {
            return Err(ValidationErrorKind::DerivativeInVertex {
                expression: id,
                function,
            });
        }
        Expression::Sample { .. } | Expression::TextureLoad { .. } | Expression::TextureQuery { .. } => {
            texture_type(program, id, expression, operands)?
        }
        _ => operation_type(id, expression, operands)?,
    };
    constant_out_of_range(&program.expressions, id, expression)?;
    Ok(Some(ty))
}

/// The type of the value at `place`, or why there is none. `index_type` gives each index's type.
pub fn place_type(
    program: &Program,
    place: &Place,
    index_type: impl Fn(ExpressionId) -> Type,
) -> Result<Type, PlaceProblem> {
    let PlaceRoot::Uniform(frequency) = place.root;
    let mut steps = place.steps().iter();
    let first = steps.next().ok_or(PlaceProblem::Empty)?;
    let Step::Field(member) = *first else {
        return Err(PlaceProblem::NotAnArray);
    };
    let mut current = program
        .uniforms_of(frequency)
        .nth(member as usize)
        .map(|(_, uniform)| uniform.ty)
        .ok_or(PlaceProblem::UndefinedMember { frequency, member })?;
    for step in steps {
        current = match (*step, current) {
            (Step::Field(field), DataType::Struct(id)) => {
                let declaration = program.structs.get(id.index()).ok_or(PlaceProblem::NotAStruct)?;
                let found = declaration.fields.get(field as usize).map(|field| field.ty);
                found.ok_or_else(|| PlaceProblem::UndefinedField {
                    structure: declaration.name.clone(),
                    field,
                })?
            }
            (Step::Index(index), DataType::Array { element, length }) => {
                let found = index_type(index);
                if !matches!(found, Type::Int | Type::UInt) {
                    return Err(PlaceProblem::IndexType { found });
                }
                let constant = match program.expressions.get(index.index()) {
                    Some(Expression::Literal(Literal::Int(value))) => Some(i64::from(*value)),
                    Some(Expression::Literal(Literal::UInt(value))) => Some(i64::from(*value)),
                    _ => None,
                };
                if let Some(index) = constant
                    && !(0..i64::from(length)).contains(&index)
                {
                    return Err(PlaceProblem::IndexOutOfRange { index, length });
                }
                element.into()
            }
            (Step::Field(_), _) => return Err(PlaceProblem::NotAStruct),
            (Step::Index(_), _) => return Err(PlaceProblem::NotAnArray),
        };
    }
    match current {
        DataType::Value(ty) => Ok(ty),
        DataType::Struct(_) | DataType::Array { .. } => Err(PlaceProblem::NotAValue),
    }
}

/// What a texture read or query of a texture of `kind` takes, as errors print it.
fn texture_signature(kind: ResourceKind, expression: &Expression) -> String {
    let layer = if kind.is_array() { ", an Int or UInt layer" } else { "" };
    match *expression {
        Expression::Sample { level, .. } => {
            let operation = level.operation();
            if kind.is_integer() {
                return format!("a float texture: a {} is read with load", kind.name());
            }
            let Some(coordinates) = kind.coordinates() else {
                return format!("{operation}(a texture, a Sampler, its coordinates)");
            };
            let tail = match level {
                SampleLevel::Implicit => "",
                SampleLevel::Bias(_) => ", a Float bias",
                SampleLevel::Explicit(_) => ", a Float level",
            };
            format!("{operation}({}, Sampler, {coordinates}{layer}{tail})", kind.name())
        }
        Expression::TextureLoad { .. } if kind == ResourceKind::TextureCube => {
            "a texture that isn't a cube: a cube is only sampled".to_owned()
        }
        Expression::TextureLoad { .. } if kind.is_texture() => {
            let coordinates = if kind.dimensions() == 3 {
                "Vec3i or Vec3u"
            } else {
                "Vec2i or Vec2u"
            };
            format!("load({}, {coordinates}{layer}, an Int or UInt level)", kind.name())
        }
        Expression::TextureLoad { .. } => "load(a texture, its texel coordinates, a level)".to_owned(),
        Expression::TextureQuery {
            query: TextureQuery::Dimensions(_),
            ..
        } => "a texture, and maybe an Int or UInt level".to_owned(),
        Expression::TextureQuery {
            query: TextureQuery::Layers,
            ..
        } => "a Texture2DArray".to_owned(),
        _ => "a texture".to_owned(),
    }
}

/// The type of a texture read or query, `expression`, whose value operands have the types
/// `operands` (coordinates, then layer, then level), or what is wrong with it.
pub fn texture_type(
    program: &Program,
    id: ExpressionId,
    expression: &Expression,
    operands: &[Type],
) -> Result<Type, ValidationErrorKind> {
    let kind = |resource: ResourceId| {
        let found = program.resources.get(resource.index()).map(|declared| declared.kind);
        found.ok_or(ValidationErrorKind::UndefinedResource {
            expression: id,
            resource,
        })
    };
    let integer = |ty: &Type| ty.is_integer() && ty.is_scalar();
    // An array texture takes one integer layer, and no other texture takes any.
    let layered = |texture: ResourceKind, layer: Option<ExpressionId>| {
        texture.is_array() == layer.is_some() && layer.is_none_or(|_| integer(&operands[1]))
    };
    let (texture, resources, ty) = match *expression {
        Expression::Sample {
            texture,
            sampler,
            layer,
            level,
            ..
        } => {
            let (texture, sampler) = (kind(texture)?, kind(sampler)?);
            let level = level.expression().map(|_| operands[operands.len() - 1]);
            let fits = sampler == ResourceKind::Sampler
                && texture.coordinates() == Some(operands[0])
                && layered(texture, layer)
                && level.is_none_or(|level| level == Type::Float);
            (texture, vec![texture.name(), sampler.name()], fits.then_some(Type::Vec4f))
        }
        Expression::TextureLoad { texture, layer, .. } => {
            let texture = kind(texture)?;
            let coordinates = operands[0];
            let fits = texture != ResourceKind::TextureCube
                && coordinates.is_integer()
                && coordinates.components() == texture.dimensions()
                && layered(texture, layer)
                && integer(&operands[operands.len() - 1]);
            (texture, vec![texture.name()], texture.texel().filter(|_| fits))
        }
        Expression::TextureQuery { texture, query } => {
            let texture = kind(texture)?;
            let ty = match query {
                TextureQuery::Dimensions(_) if texture.is_texture() && operands.iter().all(integer) => {
                    Type::vector(Scalar::UInt, texture.dimensions())
                }
                TextureQuery::Levels if texture.is_texture() => Some(Type::UInt),
                TextureQuery::Layers if texture.is_array() => Some(Type::UInt),
                _ => None,
            };
            (texture, vec![texture.name()], ty)
        }
        _ => unreachable!("{} is not a texture operation", expression.operation()),
    };
    ty.ok_or_else(|| ValidationErrorKind::InvalidTexture {
        expression: id,
        operation: expression.operation(),
        arguments: resources
            .into_iter()
            .chain(operands.iter().map(|operand| operand.name()))
            .collect(),
        expected: texture_signature(texture, expression),
    })
}

/// The type of `literal`, or the error for a NaN or infinite one.
pub fn literal_type(id: ExpressionId, literal: Literal) -> Result<Type, ValidationErrorKind> {
    if literal.floats().iter().any(|value| !value.is_finite()) {
        return Err(ValidationErrorKind::NonFiniteLiteral { expression: id });
    }
    Ok(literal.ty())
}

/// The error for arithmetic on two literals that WGSL would reject as a constant expression.
pub fn constant_out_of_range(
    expressions: &[Expression],
    id: ExpressionId,
    expression: &Expression,
) -> Result<(), ValidationErrorKind> {
    if let Some((operator, left, right)) = Arithmetic::of(expression)
        && let (Expression::Literal(left), Expression::Literal(right)) =
            (expressions[left.index()], expressions[right.index()])
        && evaluate(operator, left, right).is_none()
    {
        return Err(ValidationErrorKind::ConstantOutOfRange { expression: id });
    }
    if let Expression::Binary(BinaryOperator::ShiftLeft | BinaryOperator::ShiftRight, _, amount) = *expression
        && let Expression::Literal(Literal::UInt(amount)) = expressions[amount.index()]
        && amount >= 32
    {
        return Err(ValidationErrorKind::ShiftTooFar { expression: id, amount });
    }
    Ok(())
}

/// What a constructor of `target` accepts; `None` when `target` is a scalar.
pub fn construct_expected(target: Type) -> Option<&'static str> {
    match target {
        Type::Vec2f => Some("floats and float vectors adding up to 2 components, or one Float"),
        Type::Vec3f => Some("floats and float vectors adding up to 3 components, or one Float"),
        Type::Vec4f => Some("floats and float vectors adding up to 4 components, or one Float"),
        Type::Vec2i => Some("Ints and Int vectors adding up to 2 components, or one Int"),
        Type::Vec3i => Some("Ints and Int vectors adding up to 3 components, or one Int"),
        Type::Vec4i => Some("Ints and Int vectors adding up to 4 components, or one Int"),
        Type::Vec2u => Some("UInts and UInt vectors adding up to 2 components, or one UInt"),
        Type::Vec3u => Some("UInts and UInt vectors adding up to 3 components, or one UInt"),
        Type::Vec4u => Some("UInts and UInt vectors adding up to 4 components, or one UInt"),
        Type::Mat3f => Some("three Vec3f columns"),
        Type::Mat4f => Some("four Vec4f columns"),
        _ => None,
    }
}

/// The type of a non-leaf `expression` whose operands have the types `operands`, in order, or
/// what is wrong with that combination. Stage rules (where a texture may be sampled) are not
/// checked here: they need the whole program.
#[allow(clippy::too_many_lines)] // one arm per operation
pub fn operation_type(id: ExpressionId, expression: &Expression, operands: &[Type]) -> Result<Type, ValidationErrorKind> {
    let invalid = |expected: &'static str| ValidationErrorKind::InvalidOperation {
        expression: id,
        operation: expression.operation(),
        operands: operands.to_vec(),
        expected,
    };
    let ty = match *expression {
        Expression::Literal(_)
        | Expression::Attribute(_)
        | Expression::Builtin(_)
        | Expression::Varying(_)
        | Expression::Load(_)
        | Expression::Sample { .. }
        | Expression::TextureLoad { .. }
        | Expression::TextureQuery { .. } => unreachable!("typed by their declarations, not their operands"),
        Expression::Binary(operator, ..) => {
            let (left, right) = (operands[0], operands[1]);
            let (ty, expected) = match operator {
                BinaryOperator::Add | BinaryOperator::Subtract if left == right && left.is_matrix() => (Some(left), ""),
                BinaryOperator::Add | BinaryOperator::Subtract => (arithmetic(left, right), ADD_SUBTRACT),
                BinaryOperator::Divide | BinaryOperator::Remainder => (arithmetic(left, right), ARITHMETIC),
                BinaryOperator::Multiply => (multiply(left, right), MULTIPLY),
                BinaryOperator::Equal | BinaryOperator::NotEqual => {
                    let same = left == right && left.is_scalar();
                    (same.then_some(Type::Bool), "two scalars of the same type")
                }
                BinaryOperator::Less
                | BinaryOperator::LessEqual
                | BinaryOperator::Greater
                | BinaryOperator::GreaterEqual => {
                    let same = left == right && left.is_numeric_scalar();
                    (same.then_some(Type::Bool), "two Ints, UInts, or Floats of the same type")
                }
                BinaryOperator::And | BinaryOperator::Or => {
                    let both = left == Type::Bool && right == Type::Bool;
                    (both.then_some(Type::Bool), "two Bools")
                }
                BinaryOperator::BitAnd | BinaryOperator::BitOr | BinaryOperator::BitXor => {
                    let same = left == right && left.is_integer();
                    (same.then_some(left), "two Int or UInt values of the same type")
                }
                BinaryOperator::ShiftLeft | BinaryOperator::ShiftRight => {
                    let amount = Type::vector(Scalar::UInt, left.components());
                    let shifts = left.is_integer() && amount == Some(right);
                    (shifts.then_some(left), "an Int or UInt value, and a UInt amount of its size")
                }
            };
            ty.ok_or_else(|| invalid(expected))?
        }
        Expression::Unary(operator, _) => {
            let value = operands[0];
            let (accepts, expected) = match operator {
                UnaryOperator::Not => (value == Type::Bool, "a Bool"),
                UnaryOperator::Negate => (
                    value.is_numeric() && value.scalar() != Scalar::UInt,
                    "a signed or float scalar or vector",
                ),
                UnaryOperator::BitNot => (value.is_integer(), "an Int or UInt value"),
            };
            if !accepts {
                return Err(invalid(expected));
            }
            value
        }
        Expression::Select { .. } => {
            let (condition, accept, reject) = (operands[0], operands[1], operands[2]);
            if condition != Type::Bool || accept != reject || accept.is_matrix() {
                return Err(invalid("a Bool, then two scalars or vectors of the same type"));
            }
            accept
        }
        Expression::Call { function, .. } => {
            if operands.len() != function.arity() {
                return Err(invalid(match function.arity() {
                    1 => "one argument",
                    2 => "two arguments",
                    _ => "three arguments",
                }));
            }
            function_type(function, operands).map_err(invalid)?
        }
        Expression::Construct { target, .. } => {
            let expected = construct_expected(target).ok_or_else(|| invalid(CONSTRUCT_TARGET))?;
            if let Some(column) = target.column() {
                if operands.len() != usize::from(column.components()) || operands.iter().any(|operand| *operand != column) {
                    return Err(invalid(expected));
                }
                return Ok(target);
            }
            let splat = operands.len() == 1 && Some(operands[0]) == target.component();
            let of_its_kind = operands
                .iter()
                .all(|operand| operand.scalar() == target.scalar() && operand.components() > 0);
            let size: u8 = operands.iter().map(|operand| operand.components()).sum();
            if !splat && (!of_its_kind || size != target.components()) {
                return Err(invalid(expected));
            }
            target
        }
        Expression::Convert { target, .. } => {
            let value = operands[0];
            if target.is_matrix() || value.is_matrix() || target.components() != value.components() {
                return Err(invalid("a scalar or vector of the same size"));
            }
            target
        }
        Expression::Swizzle { swizzle, .. } => {
            let vector = operands[0];
            let size = vector.components();
            let read = u8::try_from(swizzle.components().len())
                .ok()
                .and_then(|count| Type::vector(vector.scalar(), count));
            match read {
                Some(ty) if vector.is_vector() && swizzle.components().iter().all(|component| component.index() < size) => {
                    ty
                }
                _ => {
                    return Err(ValidationErrorKind::InvalidSwizzle {
                        expression: id,
                        vector,
                        swizzle,
                    });
                }
            }
        }
    };
    Ok(ty)
}

/// The type of calling `function` with arguments of the types `operands`, as many as it takes,
/// or what it accepts.
fn function_type(function: Function, operands: &[Type]) -> Result<Type, &'static str> {
    let float = |ty: Type| ty.float_components() > 0;
    // Its type, or a Float where `value` is a float vector: what WGSL is given splatted.
    let float_beside = |ty: Type, value: Type| ty == value || (ty == Type::Float && value.is_float_vector());
    let ty = match function {
        Function::Min | Function::Max => arithmetic(operands[0], operands[1]).ok_or(ARITHMETIC)?,
        Function::Clamp => {
            let value = operands[0];
            let accepts = |bound: Type| bound == value || Some(bound) == value.component();
            if !value.is_numeric() || !accepts(operands[1]) || !accepts(operands[2]) {
                return Err("a numeric value and two bounds of its type, or a vector and two scalars of its kind");
            }
            value
        }
        Function::Abs => {
            if !operands[0].is_numeric() {
                return Err("a numeric scalar or vector");
            }
            operands[0]
        }
        Function::Sign => {
            if !operands[0].is_numeric() || operands[0].scalar() == Scalar::UInt {
                return Err("a signed or float scalar or vector");
            }
            operands[0]
        }
        Function::Floor
        | Function::Ceil
        | Function::Round
        | Function::Trunc
        | Function::Fract
        | Function::Sqrt
        | Function::InverseSqrt
        | Function::Exp
        | Function::Exp2
        | Function::Log
        | Function::Log2
        | Function::Sin
        | Function::Cos
        | Function::Tan
        | Function::Asin
        | Function::Acos
        | Function::Atan
        | Function::Radians
        | Function::Degrees
        | Function::Saturate
        | Function::Dpdx
        | Function::Dpdy
        | Function::Fwidth => {
            if !float(operands[0]) {
                return Err(FLOAT);
            }
            operands[0]
        }
        Function::Pow | Function::Atan2 | Function::Step => {
            let (left, right) = (operands[0], operands[1]);
            let value = if left.is_float_vector() { left } else { right };
            if !float(value) || !float_beside(left, value) || !float_beside(right, value) {
                return Err("two Floats or float vectors of the same type, or a float vector and a Float");
            }
            value
        }
        Function::Smoothstep => {
            let (low, high, value) = (operands[0], operands[1], operands[2]);
            if !float(value) || !float_beside(low, value) || !float_beside(high, value) {
                return Err("two edges and a Float or float vector, each edge of its type or a Float");
            }
            value
        }
        Function::Mix => {
            let (start, end, amount) = (operands[0], operands[1], operands[2]);
            if start != end || !float(start) || (amount != start && amount != Type::Float) {
                return Err("two Floats or float vectors of the same type, and an amount of that type or Float");
            }
            start
        }
        Function::Dot
        | Function::Cross
        | Function::Length
        | Function::Distance
        | Function::Normalize
        | Function::Reflect
        | Function::Refract => return geometric_type(function, operands),
        Function::Transpose | Function::Determinant => {
            if !operands[0].is_matrix() {
                return Err("a matrix");
            }
            if function == Function::Determinant {
                Type::Float
            } else {
                operands[0]
            }
        }
    };
    Ok(ty)
}

/// The type of calling a vector function, `dot` to `refract`, or what it accepts.
fn geometric_type(function: Function, operands: &[Type]) -> Result<Type, &'static str> {
    let float = |ty: Type| ty.float_components() > 0;
    let ty = match function {
        Function::Length => {
            if !float(operands[0]) {
                return Err(FLOAT);
            }
            Type::Float
        }
        Function::Dot => operands[0]
            .component()
            .filter(|_| operands[0] == operands[1])
            .ok_or("two vectors of the same type")?,
        Function::Cross => {
            if operands != [Type::Vec3f, Type::Vec3f] {
                return Err("two Vec3f");
            }
            Type::Vec3f
        }
        Function::Distance => {
            if operands[0] != operands[1] || !float(operands[0]) {
                return Err("two Floats or float vectors of the same type");
            }
            Type::Float
        }
        Function::Normalize => {
            if !operands[0].is_float_vector() {
                return Err("a float vector");
            }
            operands[0]
        }
        Function::Reflect => {
            if operands[0] != operands[1] || !operands[0].is_float_vector() {
                return Err("two float vectors of the same type");
            }
            operands[0]
        }
        Function::Refract => {
            if operands[0] != operands[1] || !operands[0].is_float_vector() || operands[2] != Type::Float {
                return Err("two float vectors of the same type and a Float");
            }
            operands[0]
        }
        _ => unreachable!("{} is not a vector function", function.name()),
    };
    Ok(ty)
}

/// `+ - / min max` on non-matrices: same numeric type, or a vector and a scalar of its kind.
fn arithmetic(left: Type, right: Type) -> Option<Type> {
    if left == right && left.is_numeric() {
        return Some(left);
    }
    if left.component() == Some(right) {
        return Some(left);
    }
    if right.component() == Some(left) {
        return Some(right);
    }
    None
}

fn multiply(left: Type, right: Type) -> Option<Type> {
    if let Some(ty) = arithmetic(left, right) {
        return Some(ty);
    }
    match (left, right) {
        (Type::Mat3f | Type::Float, Type::Mat3f) | (Type::Mat3f, Type::Float) => Some(Type::Mat3f),
        (Type::Mat4f | Type::Float, Type::Mat4f) | (Type::Mat4f, Type::Float) => Some(Type::Mat4f),
        (Type::Mat3f, Type::Vec3f) | (Type::Vec3f, Type::Mat3f) => Some(Type::Vec3f),
        (Type::Mat4f, Type::Vec4f) | (Type::Vec4f, Type::Mat4f) => Some(Type::Vec4f),
        _ => None,
    }
}

fn check_outputs(program: &Program, types: &[Option<Type>], varyings_read: &[bool], errors: &mut Vec<ValidationError>) {
    let type_of = |expression: ExpressionId| types.get(expression.index()).copied().flatten();
    let origin_of = |expression: ExpressionId| program.origin(expression).cloned();

    if let Some(position) = program.position
        && let Some(found) = type_of(position)
        && found != Type::Vec4f
    {
        errors.push(ValidationError {
            kind: ValidationErrorKind::PositionType { found },
            origin: origin_of(position),
        });
    }
    let mut written = vec![false; program.varyings.len()];
    for &(varying, expression) in &program.varying_writes {
        let Some(declaration) = program.varyings.get(varying.index()) else {
            continue;
        };
        written[varying.index()] = true;
        let Some(ty) = type_of(expression) else {
            continue;
        };
        if ty != declaration.ty {
            errors.push(ValidationError {
                kind: ValidationErrorKind::VaryingType {
                    varying: declaration.name.clone(),
                    declared: declaration.ty,
                    written: ty,
                },
                origin: origin_of(expression),
            });
        }
    }
    for (location, &output) in (0..).zip(&program.fragment_outputs) {
        let Some(found) = type_of(output) else {
            continue;
        };
        if found != Type::Vec4f {
            errors.push(ValidationError {
                kind: ValidationErrorKind::FragmentOutputType { location, found },
                origin: origin_of(output),
            });
        }
    }
    for (index, declaration) in program.varyings.iter().enumerate() {
        if varyings_read[index] && !written[index] {
            errors.push(ValidationError {
                kind: ValidationErrorKind::VaryingNotWritten {
                    varying: declaration.name.clone(),
                },
                origin: declaration.origin.clone(),
            });
        }
    }
}
