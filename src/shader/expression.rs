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
}

impl BinaryOperator {
    /// The operator as errors and WGSL write it: `+`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
        }
    }
}

/// A built-in function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Function {
    /// The smaller of two values, per component.
    Min,
    /// The larger of two values, per component.
    Max,
    /// `value` clamped to `low..=high`, per component.
    Clamp,
    /// The dot product of two vectors of one type.
    Dot,
    /// The cross product of two `Vec3f`.
    Cross,
    /// A vector scaled to length 1.
    Normalize,
    /// A vector's length, or a float's absolute value.
    Length,
    /// The greatest integer no larger than a float or each component of a float vector.
    Floor,
    /// `start + (end - start) * amount`, per component.
    Mix,
}

impl Function {
    /// The function's name, as errors and WGSL write it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Function::Min => "min",
            Function::Max => "max",
            Function::Clamp => "clamp",
            Function::Dot => "dot",
            Function::Cross => "cross",
            Function::Normalize => "normalize",
            Function::Length => "length",
            Function::Floor => "floor",
            Function::Mix => "mix",
        }
    }

    /// How many arguments it takes.
    #[must_use]
    pub fn arity(self) -> usize {
        match self {
            Function::Normalize | Function::Length | Function::Floor => 1,
            Function::Min | Function::Max | Function::Dot | Function::Cross => 2,
            Function::Clamp | Function::Mix => 3,
        }
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
    /// An operator between two values.
    Binary(BinaryOperator, ExpressionId, ExpressionId),
    /// A built-in function.
    Call {
        /// The function.
        function: Function,
        /// Its arguments, as many as [`Function::arity`].
        arguments: Operands,
    },
    /// A float vector from floats and smaller vectors whose sizes add up to the target's, or
    /// from one float repeated.
    Construct {
        /// `Vec2f`, `Vec3f`, or `Vec4f`.
        target: Type,
        /// The components, in order.
        components: Operands,
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
            Expression::Binary(_, left, right) => &[*left, *right],
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
            Expression::Binary(_, left, right) => {
                rewrite(left);
                rewrite(right);
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
            Expression::Binary(operator, ..) => operator.name(),
            Expression::Call { function, .. } => function.name(),
            Expression::Construct { target, .. } => target.name(),
            Expression::Swizzle { .. } => "swizzle",
        }
    }
}
