//! Type checking and stage checks, in Jess terms, before anything is lowered.

use std::fmt;

use crate::shader::expression::{
    Expression, ExpressionId, Literal, ParameterId, Semantic, SourceOrigin, Swizzle, VaryingId, VertexInputId,
};
use crate::shader::fold::{Arithmetic, evaluate};
use crate::shader::program::{Declaration, Program};
use crate::shader::types::Type;

/// The most color outputs a fragment stage can have.
pub const MAX_FRAGMENT_OUTPUTS: usize = 8;

pub const VERTEX: u8 = 1;
pub const FRAGMENT: u8 = 2;

/// What kind of thing a declaration declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeclarationKind {
    /// A parameter.
    Parameter,
    /// A vertex input.
    VertexInput,
    /// A varying.
    Varying,
}

impl DeclarationKind {
    fn name(self) -> &'static str {
        match self {
            DeclarationKind::Parameter => "parameter",
            DeclarationKind::VertexInput => "vertex input",
            DeclarationKind::Varying => "varying",
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
    /// An expression reads a parameter that was never declared.
    UndefinedParameter {
        /// The reader.
        expression: ExpressionId,
        /// The parameter.
        parameter: ParameterId,
    },
    /// An expression reads a vertex input that was never declared.
    UndefinedVertexInput {
        /// The reader.
        expression: ExpressionId,
        /// The vertex input.
        input: VertexInputId,
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
    /// A swizzle reads a component its vector does not have, or reads a non-vector.
    InvalidSwizzle {
        /// The swizzle.
        expression: ExpressionId,
        /// The type swizzled.
        vector: Type,
        /// The components.
        swizzle: Swizzle,
    },
    /// The fragment stage reads a vertex input; it only sees varyings.
    VertexInputInFragment {
        /// The reader.
        expression: ExpressionId,
        /// The vertex input's name.
        input: String,
    },
    /// The vertex stage reads a varying; it only writes them.
    VaryingInVertex {
        /// The reader.
        expression: ExpressionId,
        /// The varying's name.
        varying: String,
    },
    /// The vertex stage samples a texture; sampling needs the fragment stage's derivatives.
    SampleInVertex {
        /// The sample.
        expression: ExpressionId,
    },
    /// A semantic the lowering cannot provide yet (the pass-specific inputs).
    UnsupportedSemantic {
        /// The reader.
        expression: ExpressionId,
        /// The semantic.
        semantic: Semantic,
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
                "{} name {name:?} is invalid: expected ASCII letters, digits, and underscores, not starting with a digit",
                kind.name()
            ),
            ValidationErrorKind::DuplicateName { kind, name } => {
                write!(formatter, "{} {name:?} is declared twice", kind.name())
            }
            ValidationErrorKind::UnsupportedDeclarationType { kind, name, ty } => {
                let expected = match kind {
                    DeclarationKind::Parameter => "Int, UInt, Float, a vector, a matrix, Texture2D, or Sampler",
                    DeclarationKind::VertexInput | DeclarationKind::Varying => "Int, UInt, Float, or a vector",
                };
                write!(formatter, "{} {name:?} cannot be {ty}: expected {expected}", kind.name())
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
            ValidationErrorKind::UndefinedParameter { expression, parameter } => write!(
                formatter,
                "expression {} reads parameter {}, which is not declared",
                expression.index(),
                parameter.index()
            ),
            ValidationErrorKind::UndefinedVertexInput { expression, input } => write!(
                formatter,
                "expression {} reads vertex input {}, which is not declared",
                expression.index(),
                input.index()
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
                    && matches!(*operation, "+" | "-" | "*" | "/")
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
            ValidationErrorKind::InvalidSwizzle { vector, swizzle, .. } => {
                write!(formatter, "{vector}.{swizzle} is invalid: expected components a {vector} has")
            }
            ValidationErrorKind::VertexInputInFragment { input, .. } => write!(
                formatter,
                "vertex input {input:?} is read in the fragment stage: pass it through a varying"
            ),
            ValidationErrorKind::VaryingInVertex { varying, .. } => write!(
                formatter,
                "varying {varying:?} is read in the vertex stage, which only writes varyings"
            ),
            ValidationErrorKind::SampleInVertex { .. } => {
                formatter.write_str("sample is used in the vertex stage: textures are sampled in the fragment stage")
            }
            ValidationErrorKind::UnsupportedSemantic { semantic, .. } => {
                write!(formatter, "semantic {semantic} is not supported by the WGSL lowering yet")
            }
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
    let kinds = [
        (DeclarationKind::Parameter, &program.parameters),
        (DeclarationKind::VertexInput, &program.vertex_inputs),
        (DeclarationKind::Varying, &program.varyings),
    ];
    for (kind, declarations) in kinds {
        for (index, declaration) in declarations.iter().enumerate() {
            let Declaration { name, ty, origin } = declaration;
            for kind in declaration_errors(kind, &declarations[..index], name, *ty) {
                errors.push(ValidationError {
                    kind,
                    origin: origin.clone(),
                });
            }
        }
    }
}

/// What is wrong with declaring `name` as `ty` after `earlier` declarations of the same `kind`:
/// an invalid name, a name already taken, or a type the kind cannot have. Empty when nothing is.
pub fn declaration_errors(kind: DeclarationKind, earlier: &[Declaration], name: &str, ty: Type) -> Vec<ValidationErrorKind> {
    let mut errors = Vec::new();
    if !is_valid_name(name) {
        errors.push(ValidationErrorKind::InvalidName {
            kind,
            name: name.to_owned(),
        });
    }
    // Textures, samplers, and values of the same name lower to different prefixes.
    let key = |ty: Type| (ty == Type::Texture2D, ty == Type::Sampler);
    if earlier
        .iter()
        .any(|declaration| declaration.name == name && key(declaration.ty) == key(ty))
    {
        errors.push(ValidationErrorKind::DuplicateName {
            kind,
            name: name.to_owned(),
        });
    }
    let supported = match kind {
        DeclarationKind::Parameter => ty != Type::Bool,
        DeclarationKind::VertexInput | DeclarationKind::Varying => ty.is_numeric_scalar() || ty.is_vector(),
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
pub const CONSTRUCT_TARGET: &str = "a vector target";
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
        Expression::Parameter(parameter) => {
            let Some(declaration) = program.parameters.get(parameter.index()) else {
                return Err(ValidationErrorKind::UndefinedParameter {
                    expression: id,
                    parameter,
                });
            };
            declaration.ty
        }
        Expression::VertexInput(input) => {
            let Some(declaration) = program.vertex_inputs.get(input.index()) else {
                return Err(ValidationErrorKind::UndefinedVertexInput { expression: id, input });
            };
            if stage & FRAGMENT != 0 {
                return Err(ValidationErrorKind::VertexInputInFragment {
                    expression: id,
                    input: declaration.name.clone(),
                });
            }
            declaration.ty
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
        Expression::Semantic(semantic) => {
            if semantic.ty().is_resource() {
                return Err(ValidationErrorKind::UnsupportedSemantic {
                    expression: id,
                    semantic,
                });
            }
            semantic.ty()
        }
        Expression::Sample { .. } if stage & VERTEX != 0 => {
            return Err(ValidationErrorKind::SampleInVertex { expression: id });
        }
        _ => operation_type(id, expression, operands)?,
    };
    constant_out_of_range(&program.expressions, id, expression)?;
    Ok(Some(ty))
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
    Ok(())
}

/// What a vector constructor of `target` accepts; `None` when `target` is not a vector.
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
        | Expression::Parameter(_)
        | Expression::VertexInput(_)
        | Expression::Varying(_)
        | Expression::Semantic(_) => unreachable!("leaves are typed by their declaration"),
        Expression::Sample { .. } => {
            if operands != [Type::Texture2D, Type::Sampler, Type::Vec2f] {
                return Err(invalid("sample(Texture2D, Sampler, Vec2f)"));
            }
            Type::Vec4f
        }
        Expression::Add(..) | Expression::Subtract(..) => {
            let (left, right) = (operands[0], operands[1]);
            if left == right && left.is_matrix() {
                left
            } else {
                arithmetic(left, right).ok_or_else(|| invalid(ADD_SUBTRACT))?
            }
        }
        Expression::Divide(..) | Expression::Min(..) | Expression::Max(..) => {
            arithmetic(operands[0], operands[1]).ok_or_else(|| invalid(ARITHMETIC))?
        }
        Expression::Multiply(..) => multiply(operands[0], operands[1]).ok_or_else(|| invalid(MULTIPLY))?,
        Expression::Clamp(..) => {
            let value = operands[0];
            let accepts = |bound: Type| bound == value || Some(bound) == value.component();
            if !value.is_numeric() || !accepts(operands[1]) || !accepts(operands[2]) {
                return Err(invalid(
                    "a numeric value and two bounds of its type, or a vector and two scalars of its kind",
                ));
            }
            value
        }
        Expression::Dot(..) => {
            let Some(component) = operands[0].component().filter(|_| operands[0] == operands[1]) else {
                return Err(invalid("two vectors of the same type"));
            };
            component
        }
        Expression::Cross(..) => {
            if operands != [Type::Vec3f, Type::Vec3f] {
                return Err(invalid("two Vec3f"));
            }
            Type::Vec3f
        }
        Expression::Normalize(..) => {
            if !operands[0].is_float_vector() {
                return Err(invalid("a float vector"));
            }
            operands[0]
        }
        Expression::Length(..) => {
            if operands[0].float_components() == 0 {
                return Err(invalid("a Float or float vector"));
            }
            Type::Float
        }
        Expression::Floor(..) => {
            if operands[0].float_components() == 0 {
                return Err(invalid("a Float or float vector"));
            }
            operands[0]
        }
        Expression::Mix(..) => {
            let (start, end, amount) = (operands[0], operands[1], operands[2]);
            if start != end || start.float_components() == 0 || (amount != start && amount != Type::Float) {
                return Err(invalid(
                    "two Floats or float vectors of the same type, and an amount of that type or Float",
                ));
            }
            start
        }
        Expression::Construct { target, .. } => {
            let expected = construct_expected(target).ok_or_else(|| invalid(CONSTRUCT_TARGET))?;
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
        Expression::Swizzle { swizzle, .. } => {
            let vector = operands[0];
            let size = vector.components();
            let read = vector
                .scalar()
                .zip(u8::try_from(swizzle.components().len()).ok())
                .and_then(|(scalar, count)| Type::vector(scalar, count));
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
