//! Expression nodes, the ids that name them, and where they came from.

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use crate::shader::interface::{AttributeId, Frequency, ResourceId};
use crate::shader::types::Type;

arena_id!(
    /// An expression in a [`Program`](crate::shader::Program)'s arena.
    ///
    /// Ids only point backwards: an expression's operands always come before it, so the arena is
    /// already in dependency order. Validation rejects an operand that does not.
    ExpressionId,
    "expression's position in the arena"
);
arena_id!(
    /// A varying declared with [`Program::varying`](crate::shader::Program::varying).
    VaryingId,
    "varying's position in declaration order"
);

/// Where an expression or declaration was written, for errors that point back at the author.
///
/// The owner is whoever built the program (a package, a script); file, line, and column are the
/// author's source, not anything Jess generated.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SourceOrigin {
    /// Who built the program, such as a package name.
    pub owner: Arc<str>,
    /// The source file.
    pub file: Arc<str>,
    /// The 1-based line.
    pub line: u32,
    /// The 1-based column, or 0 when the source only records lines (Luau does), which prints as
    /// `file:line`.
    pub column: u32,
}

impl fmt::Display for SourceOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.column == 0 {
            return write!(formatter, "{}:{} ({})", self.file, self.line, self.owner);
        }
        write!(formatter, "{}:{}:{} ({})", self.file, self.line, self.column, self.owner)
    }
}

/// A constant value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Literal {
    /// A `Bool`.
    Bool(bool),
    /// An `Int`.
    Int(i32),
    /// A `UInt`.
    UInt(u32),
    /// A `Float`.
    Float(f32),
    /// A `Vec2f`.
    Vec2f([f32; 2]),
    /// A `Vec3f`.
    Vec3f([f32; 3]),
    /// A `Vec4f`.
    Vec4f([f32; 4]),
}

impl Literal {
    /// The literal's type.
    #[must_use]
    pub fn ty(&self) -> Type {
        match self {
            Literal::Bool(_) => Type::Bool,
            Literal::Int(_) => Type::Int,
            Literal::UInt(_) => Type::UInt,
            Literal::Float(_) => Type::Float,
            Literal::Vec2f(_) => Type::Vec2f,
            Literal::Vec3f(_) => Type::Vec3f,
            Literal::Vec4f(_) => Type::Vec4f,
        }
    }

    /// The float components of a `Float` or float vector literal; empty for every other literal.
    #[must_use]
    pub fn floats(&self) -> &[f32] {
        match self {
            Literal::Float(value) => std::slice::from_ref(value),
            Literal::Vec2f(values) => values,
            Literal::Vec3f(values) => values,
            Literal::Vec4f(values) => values,
            _ => &[],
        }
    }
}

/// One vector component, for a [`Swizzle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Component {
    /// The first component.
    X,
    /// The second component.
    Y,
    /// The third component.
    Z,
    /// The fourth component.
    W,
}

impl Component {
    /// The component's position in the vector.
    #[must_use]
    pub fn index(self) -> u8 {
        self as u8
    }

    /// The component's letter.
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Component::X => 'x',
            Component::Y => 'y',
            Component::Z => 'z',
            Component::W => 'w',
        }
    }
}

/// One to four components read from a vector: `.w`, `.xyz`, `.zyx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Swizzle {
    components: [Component; 4],
    count: u8,
}

impl Swizzle {
    /// A swizzle of `components`; `None` unless there are one to four.
    #[must_use]
    pub fn new(components: &[Component]) -> Option<Swizzle> {
        if components.is_empty() || components.len() > 4 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)] // at most four
        let mut swizzle = Swizzle {
            components: [Component::X; 4],
            count: components.len() as u8,
        };
        swizzle.components[..components.len()].copy_from_slice(components);
        Some(swizzle)
    }

    /// A swizzle from its letters, such as `"xyz"`; `None` unless it is one to four of `xyzw`.
    #[must_use]
    pub fn parse(letters: &str) -> Option<Swizzle> {
        let mut components = [Component::X; 4];
        let mut count = 0;
        for letter in letters.bytes() {
            if count == 4 {
                return None;
            }
            components[count] = match letter {
                b'x' => Component::X,
                b'y' => Component::Y,
                b'z' => Component::Z,
                b'w' => Component::W,
                _ => return None,
            };
            count += 1;
        }
        Swizzle::new(&components[..count])
    }

    /// The components, in order.
    #[must_use]
    pub fn components(&self) -> &[Component] {
        &self.components[..self.count as usize]
    }
}

impl fmt::Display for Swizzle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for component in self.components() {
            fmt::Write::write_char(formatter, component.letter())?;
        }
        Ok(())
    }
}

/// Up to four expression ids: a vector constructor's components, or any expression's operands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Operands {
    ids: [ExpressionId; 4],
    count: u8,
}

impl Operands {
    /// `ids` as operands; `None` if there are more than four.
    #[must_use]
    pub fn new(ids: &[ExpressionId]) -> Option<Operands> {
        if ids.len() > 4 {
            return None;
        }
        Some(Operands::from_short_slice(ids))
    }

    /// `ids`, which the caller guarantees are at most four.
    fn from_short_slice(ids: &[ExpressionId]) -> Operands {
        #[allow(clippy::cast_possible_truncation)] // at most four
        let mut operands = Operands {
            ids: [ExpressionId(0); 4],
            count: ids.len() as u8,
        };
        operands.ids[..ids.len()].copy_from_slice(ids);
        operands
    }

    fn as_mut_slice(&mut self) -> &mut [ExpressionId] {
        &mut self.ids[..self.count as usize]
    }
}

impl Deref for Operands {
    type Target = [ExpressionId];

    fn deref(&self) -> &[ExpressionId] {
        &self.ids[..self.count as usize]
    }
}

/// The most steps a [`Place`] can take.
pub const MAX_PLACE_STEPS: usize = 8;
/// The most of a [`Place`]'s steps that can be indices.
pub const MAX_PLACE_INDICES: usize = 4;

/// Where a place starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceRoot {
    /// A frequency's uniform block; the first step names one of its members.
    Uniform(Frequency),
}

/// One step into a struct or an array.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    /// A uniform block's member, by its position among the frequency's members, or a struct's
    /// field, by its position among the struct's fields.
    Field(u32),
    /// An array's element at an `Int` or `UInt` index, counted from 0.
    Index(ExpressionId),
}

/// A value in a buffer: a root, then fields and indices down to one value. `pass.lights[i].color`
/// is the pass's uniform block, member `lights`, element `i`, field `color`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    /// Where the place starts.
    pub root: PlaceRoot,
    steps: [Step; MAX_PLACE_STEPS],
    count: u8,
}

impl Place {
    /// A place of `steps` from `root`; `None` with more than [`MAX_PLACE_STEPS`] steps, or more
    /// than [`MAX_PLACE_INDICES`] indices among them.
    #[must_use]
    pub fn new(root: PlaceRoot, steps: &[Step]) -> Option<Place> {
        let indices = steps.iter().filter(|step| matches!(step, Step::Index(_))).count();
        if steps.len() > MAX_PLACE_STEPS || indices > MAX_PLACE_INDICES {
            return None;
        }
        let mut place = Place {
            root,
            steps: [Step::Field(0); MAX_PLACE_STEPS],
            #[allow(clippy::cast_possible_truncation)] // at most MAX_PLACE_STEPS
            count: steps.len() as u8,
        };
        place.steps[..steps.len()].copy_from_slice(steps);
        Some(place)
    }

    /// The place one step further in; `None` past the limits of [`Place::new`].
    #[must_use]
    pub fn then(&self, step: Step) -> Option<Place> {
        let mut steps = self.steps().to_vec();
        steps.push(step);
        Place::new(self.root, &steps)
    }

    /// The steps, in order.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps[..self.count as usize]
    }

    /// The index expressions, in order.
    #[must_use]
    pub fn indices(&self) -> Operands {
        let mut operands = Operands::from_short_slice(&[]);
        for step in self.steps() {
            if let Step::Index(index) = step {
                operands.ids[operands.count as usize] = *index;
                operands.count += 1;
            }
        }
        operands
    }

    fn rewrite_indices(&mut self, mut rewrite: impl FnMut(&mut ExpressionId)) {
        for step in &mut self.steps[..self.count as usize] {
            if let Step::Index(index) = step {
                rewrite(index);
            }
        }
    }
}

/// An operator between two values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    /// `left + right`.
    Add,
    /// `left - right`.
    Subtract,
    /// `left * right`, including matrix × vector and matrix × matrix.
    Multiply,
    /// `left / right`.
    Divide,
    /// Whether two scalars are equal.
    Equal,
    /// Whether two scalars differ.
    NotEqual,
    /// `left < right`, between two numeric scalars.
    Less,
    /// `left <= right`.
    LessEqual,
    /// `left > right`.
    Greater,
    /// `left >= right`.
    GreaterEqual,
    /// Whether two `Bool`s both hold.
    And,
    /// Whether either of two `Bool`s holds.
    Or,
    /// `left % right`: what's left of truncated division, with the sign of `left`.
    Remainder,
    /// Bitwise and, of two integer values of one type.
    BitAnd,
    /// Bitwise or.
    BitOr,
    /// Bitwise exclusive or.
    BitXor,
    /// `left` shifted left by a `UInt` amount, or a `UInt` vector of its size.
    ShiftLeft,
    /// `left` shifted right: arithmetically for `Int`, logically for `UInt`.
    ShiftRight,
}

impl BinaryOperator {
    /// The operator's Jess name, as errors print it: `+` for the arithmetic, which is written between
    /// its operands, and the function's name (`less`, `bitAnd`) for the rest.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
            BinaryOperator::Equal => "equal",
            BinaryOperator::NotEqual => "notEqual",
            BinaryOperator::Less => "less",
            BinaryOperator::LessEqual => "lessEqual",
            BinaryOperator::Greater => "greater",
            BinaryOperator::GreaterEqual => "greaterEqual",
            BinaryOperator::And => "logicalAnd",
            BinaryOperator::Or => "logicalOr",
            BinaryOperator::Remainder => "%",
            BinaryOperator::BitAnd => "bitAnd",
            BinaryOperator::BitOr => "bitOr",
            BinaryOperator::BitXor => "bitXor",
            BinaryOperator::ShiftLeft => "shiftLeft",
            BinaryOperator::ShiftRight => "shiftRight",
        }
    }

    /// The operator as WGSL writes it.
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
            BinaryOperator::Equal => "==",
            BinaryOperator::NotEqual => "!=",
            BinaryOperator::Less => "<",
            BinaryOperator::LessEqual => "<=",
            BinaryOperator::Greater => ">",
            BinaryOperator::GreaterEqual => ">=",
            BinaryOperator::And => "&&",
            BinaryOperator::Or => "||",
            BinaryOperator::Remainder => "%",
            BinaryOperator::BitAnd => "&",
            BinaryOperator::BitOr => "|",
            BinaryOperator::BitXor => "^",
            BinaryOperator::ShiftLeft => "<<",
            BinaryOperator::ShiftRight => ">>",
        }
    }
}

/// An operator on one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    /// Whether a `Bool` doesn't hold.
    Not,
    /// `-value`, of a signed or float scalar or vector.
    Negate,
    /// Every bit of an integer value flipped.
    BitNot,
}

impl UnaryOperator {
    /// The operator's Jess name, as errors print it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            UnaryOperator::Not => "logicalNot",
            UnaryOperator::Negate => "-",
            UnaryOperator::BitNot => "bitNot",
        }
    }

    /// The operator as WGSL writes it.
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            UnaryOperator::Not => "!",
            UnaryOperator::Negate => "-",
            UnaryOperator::BitNot => "~",
        }
    }
}

/// A built-in function. Its name is the one Luau and WGSL both give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Function {
    /// The smaller of two values, per component.
    Min,
    /// The larger of two values, per component.
    Max,
    /// `value` clamped to `low..=high`, per component.
    Clamp,
    /// The absolute value, per component.
    Abs,
    /// -1, 0, or 1, per component, as the value is negative, zero, or positive.
    Sign,
    /// The greatest integer no larger than a float, per component.
    Floor,
    /// The least integer no smaller than a float, per component.
    Ceil,
    /// The nearest integer, per component; halves round to even.
    Round,
    /// A float with its fraction dropped, per component: toward zero.
    Trunc,
    /// `x - floor(x)`, per component.
    Fract,
    /// The square root, per component.
    Sqrt,
    /// One over the square root, per component.
    InverseSqrt,
    /// `e` to the power, per component.
    Exp,
    /// 2 to the power, per component.
    Exp2,
    /// The natural logarithm, per component.
    Log,
    /// The base-2 logarithm, per component.
    Log2,
    /// `base` to the power `exponent`, per component.
    Pow,
    /// The sine of radians, per component.
    Sin,
    /// The cosine of radians, per component.
    Cos,
    /// The tangent of radians, per component.
    Tan,
    /// The arcsine, in radians, per component.
    Asin,
    /// The arccosine, in radians, per component.
    Acos,
    /// The arctangent, in radians, per component.
    Atan,
    /// The angle of the point `(x, y)`, `atan2(y, x)`, in radians from -π to π, per component.
    Atan2,
    /// Degrees in radians, per component.
    Radians,
    /// Radians in degrees, per component.
    Degrees,
    /// A float clamped to `0..=1`, per component.
    Saturate,
    /// 0 where `x < edge`, else 1, per component.
    Step,
    /// 0 at `low`, 1 at `high`, a Hermite curve between, per component.
    Smoothstep,
    /// `start + (end - start) * amount`, per component.
    Mix,
    /// The dot product of two vectors of one type.
    Dot,
    /// The cross product of two `Vec3f`.
    Cross,
    /// A vector's length, or a float's absolute value.
    Length,
    /// The length of the difference of two floats or float vectors.
    Distance,
    /// A vector scaled to length 1.
    Normalize,
    /// `incident` reflected about the plane of `normal`, a unit vector.
    Reflect,
    /// `incident` refracted through the plane of `normal` at the ratio of indices `eta`.
    Refract,
    /// A matrix's rows as its columns.
    Transpose,
    /// A matrix's determinant.
    Determinant,
    /// How fast a value changes across the screen horizontally. Fragment stage only.
    Dpdx,
    /// How fast a value changes across the screen vertically. Fragment stage only.
    Dpdy,
    /// `abs(dpdx(value)) + abs(dpdy(value))`. Fragment stage only.
    Fwidth,
}

impl Function {
    /// Every function, in the order the docs list them.
    pub const ALL: [Function; 42] = [
        Function::Min,
        Function::Max,
        Function::Clamp,
        Function::Abs,
        Function::Sign,
        Function::Floor,
        Function::Ceil,
        Function::Round,
        Function::Trunc,
        Function::Fract,
        Function::Sqrt,
        Function::InverseSqrt,
        Function::Exp,
        Function::Exp2,
        Function::Log,
        Function::Log2,
        Function::Pow,
        Function::Sin,
        Function::Cos,
        Function::Tan,
        Function::Asin,
        Function::Acos,
        Function::Atan,
        Function::Atan2,
        Function::Radians,
        Function::Degrees,
        Function::Saturate,
        Function::Step,
        Function::Smoothstep,
        Function::Mix,
        Function::Dot,
        Function::Cross,
        Function::Length,
        Function::Distance,
        Function::Normalize,
        Function::Reflect,
        Function::Refract,
        Function::Transpose,
        Function::Determinant,
        Function::Dpdx,
        Function::Dpdy,
        Function::Fwidth,
    ];

    /// The function's name, as Luau, errors, and WGSL write it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Function::Min => "min",
            Function::Max => "max",
            Function::Clamp => "clamp",
            Function::Abs => "abs",
            Function::Sign => "sign",
            Function::Floor => "floor",
            Function::Ceil => "ceil",
            Function::Round => "round",
            Function::Trunc => "trunc",
            Function::Fract => "fract",
            Function::Sqrt => "sqrt",
            Function::InverseSqrt => "inverseSqrt",
            Function::Exp => "exp",
            Function::Exp2 => "exp2",
            Function::Log => "log",
            Function::Log2 => "log2",
            Function::Pow => "pow",
            Function::Sin => "sin",
            Function::Cos => "cos",
            Function::Tan => "tan",
            Function::Asin => "asin",
            Function::Acos => "acos",
            Function::Atan => "atan",
            Function::Atan2 => "atan2",
            Function::Radians => "radians",
            Function::Degrees => "degrees",
            Function::Saturate => "saturate",
            Function::Step => "step",
            Function::Smoothstep => "smoothstep",
            Function::Mix => "mix",
            Function::Dot => "dot",
            Function::Cross => "cross",
            Function::Length => "length",
            Function::Distance => "distance",
            Function::Normalize => "normalize",
            Function::Reflect => "reflect",
            Function::Refract => "refract",
            Function::Transpose => "transpose",
            Function::Determinant => "determinant",
            Function::Dpdx => "dpdx",
            Function::Dpdy => "dpdy",
            Function::Fwidth => "fwidth",
        }
    }

    /// How many arguments it takes.
    #[must_use]
    pub fn arity(self) -> usize {
        match self {
            Function::Min
            | Function::Max
            | Function::Pow
            | Function::Atan2
            | Function::Step
            | Function::Dot
            | Function::Cross
            | Function::Distance
            | Function::Reflect => 2,
            Function::Clamp | Function::Smoothstep | Function::Mix | Function::Refract => 3,
            _ => 1,
        }
    }

    /// Whether it reads the fragment stage's derivatives, which the vertex stage has none of.
    #[must_use]
    pub fn needs_derivatives(self) -> bool {
        matches!(self, Function::Dpdx | Function::Dpdy | Function::Fwidth)
    }
}

/// A node in a [`Program`](crate::shader::Program)'s expression arena.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expression {
    /// A constant.
    Literal(Literal),
    /// A value in a uniform block.
    Load(Place),
    /// A declared vertex attribute, per vertex or per instance. Vertex stage only.
    Attribute(AttributeId),
    /// A declared varying, as the vertex stage wrote it and the rasterizer interpolated it.
    /// Fragment stage only.
    Varying(VaryingId),
    /// A texture read at `Vec2f` coordinates, to `Vec4f`. Fragment stage only.
    Sample {
        /// A `Texture2D`.
        texture: ResourceId,
        /// A `Sampler`.
        sampler: ResourceId,
        /// The `Vec2f` coordinates.
        coordinates: ExpressionId,
    },
    /// An operator on one value.
    Unary(UnaryOperator, ExpressionId),
    /// An operator between two values.
    Binary(BinaryOperator, ExpressionId, ExpressionId),
    /// `accept` where `condition` holds, else `reject`: both are computed.
    Select {
        /// A `Bool`.
        condition: ExpressionId,
        /// The value where it holds.
        accept: ExpressionId,
        /// The value where it doesn't, of the same type.
        reject: ExpressionId,
    },
    /// A built-in function.
    Call {
        /// The function.
        function: Function,
        /// Its arguments, as many as [`Function::arity`].
        arguments: Operands,
    },
    /// A vector from scalars and smaller vectors of its scalar whose sizes add up to its, or from
    /// one scalar repeated; or a matrix from its columns.
    Construct {
        /// A vector or matrix type.
        target: Type,
        /// The components, in order.
        components: Operands,
    },
    /// A scalar or vector as `target`, a type of the same size: `f32(index)`.
    Convert {
        /// The type converted to.
        target: Type,
        /// The value.
        value: ExpressionId,
    },
    /// Components read from a vector.
    Swizzle {
        /// The vector.
        vector: ExpressionId,
        /// The components.
        swizzle: Swizzle,
    },
}

impl Expression {
    /// A vector constructor; `None` if there are more than four components.
    #[must_use]
    pub fn construct(target: Type, components: &[ExpressionId]) -> Option<Expression> {
        Operands::new(components).map(|components| Expression::Construct { target, components })
    }

    /// A call of `function`; `None` if there are more than four arguments.
    #[must_use]
    pub fn call(function: Function, arguments: &[ExpressionId]) -> Option<Expression> {
        Operands::new(arguments).map(|arguments| Expression::Call { function, arguments })
    }

    /// The expressions this one reads, in order.
    #[must_use]
    pub fn operands(&self) -> Operands {
        let ids: &[ExpressionId] = match self {
            Expression::Literal(_) | Expression::Attribute(_) | Expression::Varying(_) => &[],
            Expression::Load(place) => return place.indices(),
            Expression::Sample { coordinates, .. } => &[*coordinates],
            Expression::Unary(_, value) | Expression::Convert { value, .. } => &[*value],
            Expression::Binary(_, left, right) => &[*left, *right],
            Expression::Select {
                condition,
                accept,
                reject,
            } => &[*condition, *accept, *reject],
            Expression::Call { arguments, .. } => return *arguments,
            Expression::Construct { components, .. } => return *components,
            Expression::Swizzle { vector, .. } => &[*vector],
        };
        Operands::from_short_slice(ids)
    }

    /// Calls `rewrite` on every operand, in order.
    pub(crate) fn rewrite_operands(&mut self, mut rewrite: impl FnMut(&mut ExpressionId)) {
        match self {
            Expression::Literal(_) | Expression::Attribute(_) | Expression::Varying(_) => {}
            Expression::Load(place) => place.rewrite_indices(rewrite),
            Expression::Sample { coordinates, .. } => rewrite(coordinates),
            Expression::Unary(_, value) | Expression::Convert { value, .. } => rewrite(value),
            Expression::Binary(_, left, right) => {
                rewrite(left);
                rewrite(right);
            }
            Expression::Select {
                condition,
                accept,
                reject,
            } => {
                rewrite(condition);
                rewrite(accept);
                rewrite(reject);
            }
            Expression::Call { arguments, .. } => arguments.as_mut_slice().iter_mut().for_each(rewrite),
            Expression::Construct { components, .. } => components.as_mut_slice().iter_mut().for_each(rewrite),
            Expression::Swizzle { vector, .. } => rewrite(vector),
        }
    }

    /// Whether the lowering writes the expression where it is read instead of computing it once:
    /// literals, attributes, varyings, and uniform values reached without an index. Each reads no
    /// other expression.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        match self {
            Expression::Literal(_) | Expression::Attribute(_) | Expression::Varying(_) => true,
            Expression::Load(place) => place.indices().is_empty(),
            _ => false,
        }
    }

    /// The Jess name of the operation, as errors print it: `+`, `dot`, `sample`.
    #[must_use]
    pub fn operation(&self) -> &'static str {
        match self {
            Expression::Literal(_) => "literal",
            Expression::Load(_) => "load",
            Expression::Attribute(_) => "attribute",
            Expression::Varying(_) => "varying",
            Expression::Sample { .. } => "sample",
            Expression::Unary(operator, _) => operator.name(),
            Expression::Binary(operator, ..) => operator.name(),
            Expression::Select { .. } => "select",
            Expression::Call { function, .. } => function.name(),
            Expression::Construct { target, .. } | Expression::Convert { target, .. } => target.name(),
            Expression::Swizzle { .. } => "swizzle",
        }
    }
}
