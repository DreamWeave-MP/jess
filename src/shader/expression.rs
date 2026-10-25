//! Expression nodes, the ids that name them, and where they came from.

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use crate::shader::interface::AttributeId;
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
    /// A parameter declared with [`Program::parameter`](crate::shader::Program::parameter).
    ParameterId,
    "parameter's position in declaration order"
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

/// A value the renderer provides, named by what it means rather than where it is bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Semantic {
    /// The object's model-to-world matrix (`Mat4f`).
    Model,
    /// The camera's world-to-view matrix (`Mat4f`).
    View,
    /// The camera's view-to-clip matrix (`Mat4f`).
    Projection,
    /// The camera position in world space (`Vec3f`).
    CameraPosition,
    /// The viewport size in pixels (`Vec2f`).
    ViewportSize,
    /// Seconds since the renderer started (`Float`).
    Time,
    /// The color the pass reads from the scene (`Texture2D`). Pass-specific.
    SceneColor,
    /// The depth the pass reads from the scene (`Texture2D`). Pass-specific.
    SceneDepth,
    /// The previous pass's output (`Texture2D`). Pass-specific.
    PreviousPass,
}

impl Semantic {
    /// Every semantic, in declaration order.
    pub const ALL: [Semantic; 9] = [
        Semantic::Model,
        Semantic::View,
        Semantic::Projection,
        Semantic::CameraPosition,
        Semantic::ViewportSize,
        Semantic::Time,
        Semantic::SceneColor,
        Semantic::SceneDepth,
        Semantic::PreviousPass,
    ];

    /// The semantic's type.
    #[must_use]
    pub fn ty(self) -> Type {
        match self {
            Semantic::Model | Semantic::View | Semantic::Projection => Type::Mat4f,
            Semantic::CameraPosition => Type::Vec3f,
            Semantic::ViewportSize => Type::Vec2f,
            Semantic::Time => Type::Float,
            Semantic::SceneColor | Semantic::SceneDepth | Semantic::PreviousPass => Type::Texture2D,
        }
    }

    /// The Jess name of the semantic, as errors print it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Semantic::Model => "model",
            Semantic::View => "view",
            Semantic::Projection => "projection",
            Semantic::CameraPosition => "cameraPosition",
            Semantic::ViewportSize => "viewportSize",
            Semantic::Time => "time",
            Semantic::SceneColor => "sceneColor",
            Semantic::SceneDepth => "sceneDepth",
            Semantic::PreviousPass => "previousPass",
        }
    }
}

impl fmt::Display for Semantic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
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

/// A node in a [`Program`](crate::shader::Program)'s expression arena.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expression {
    /// A constant.
    Literal(Literal),
    /// A declared parameter's value, texture, or sampler.
    Parameter(ParameterId),
    /// A declared vertex attribute, per vertex or per instance. Vertex stage only.
    Attribute(AttributeId),
    /// A declared varying, as the vertex stage wrote it and the rasterizer interpolated it.
    /// Fragment stage only.
    Varying(VaryingId),
    /// A value the renderer provides.
    Semantic(Semantic),
    /// A texture read: `Texture2D`, `Sampler`, `Vec2f` coordinates, to `Vec4f`. Fragment stage
    /// only.
    Sample {
        /// The `Texture2D`.
        texture: ExpressionId,
        /// The `Sampler`.
        sampler: ExpressionId,
        /// The `Vec2f` coordinates.
        coordinates: ExpressionId,
    },
    /// `left + right`.
    Add(ExpressionId, ExpressionId),
    /// `left - right`.
    Subtract(ExpressionId, ExpressionId),
    /// `left * right`, including matrix × vector and matrix × matrix.
    Multiply(ExpressionId, ExpressionId),
    /// `left / right`.
    Divide(ExpressionId, ExpressionId),
    /// The smaller of two values, per component.
    Min(ExpressionId, ExpressionId),
    /// The larger of two values, per component.
    Max(ExpressionId, ExpressionId),
    /// `value` clamped to `low..=high`, per component.
    Clamp(ExpressionId, ExpressionId, ExpressionId),
    /// The dot product of two vectors of the same size.
    Dot(ExpressionId, ExpressionId),
    /// The cross product of two `Vec3f`.
    Cross(ExpressionId, ExpressionId),
    /// A vector scaled to length 1.
    Normalize(ExpressionId),
    /// A vector's length, or a float's absolute value.
    Length(ExpressionId),
    /// The greatest integer no larger than a float or each component of a float vector.
    Floor(ExpressionId),
    /// `start + (end - start) * amount`, per component.
    Mix(ExpressionId, ExpressionId, ExpressionId),
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

    /// The expressions this one reads, in order.
    #[must_use]
    pub fn operands(&self) -> Operands {
        let ids: &[ExpressionId] = match self {
            Expression::Literal(_)
            | Expression::Parameter(_)
            | Expression::Attribute(_)
            | Expression::Varying(_)
            | Expression::Semantic(_) => &[],
            Expression::Sample {
                texture,
                sampler,
                coordinates,
            } => &[*texture, *sampler, *coordinates],
            Expression::Add(left, right)
            | Expression::Subtract(left, right)
            | Expression::Multiply(left, right)
            | Expression::Divide(left, right)
            | Expression::Min(left, right)
            | Expression::Max(left, right)
            | Expression::Dot(left, right)
            | Expression::Cross(left, right) => &[*left, *right],
            Expression::Clamp(first, second, third) | Expression::Mix(first, second, third) => &[*first, *second, *third],
            Expression::Normalize(value) | Expression::Length(value) | Expression::Floor(value) => &[*value],
            Expression::Construct { components, .. } => return *components,
            Expression::Swizzle { vector, .. } => &[*vector],
        };
        Operands::from_short_slice(ids)
    }

    /// Calls `rewrite` on every operand, in order.
    pub(crate) fn rewrite_operands(&mut self, mut rewrite: impl FnMut(&mut ExpressionId)) {
        match self {
            Expression::Literal(_)
            | Expression::Parameter(_)
            | Expression::Attribute(_)
            | Expression::Varying(_)
            | Expression::Semantic(_) => {}
            Expression::Sample {
                texture,
                sampler,
                coordinates,
            } => {
                rewrite(texture);
                rewrite(sampler);
                rewrite(coordinates);
            }
            Expression::Add(left, right)
            | Expression::Subtract(left, right)
            | Expression::Multiply(left, right)
            | Expression::Divide(left, right)
            | Expression::Min(left, right)
            | Expression::Max(left, right)
            | Expression::Dot(left, right)
            | Expression::Cross(left, right) => {
                rewrite(left);
                rewrite(right);
            }
            Expression::Clamp(first, second, third) | Expression::Mix(first, second, third) => {
                rewrite(first);
                rewrite(second);
                rewrite(third);
            }
            Expression::Normalize(value) | Expression::Length(value) | Expression::Floor(value) => rewrite(value),
            Expression::Construct { components, .. } => components.as_mut_slice().iter_mut().for_each(rewrite),
            Expression::Swizzle { vector, .. } => rewrite(vector),
        }
    }

    /// Whether the expression reads no other expression.
    #[must_use]
    pub fn is_leaf(&self) -> bool {
        matches!(
            self,
            Expression::Literal(_)
                | Expression::Parameter(_)
                | Expression::Attribute(_)
                | Expression::Varying(_)
                | Expression::Semantic(_)
        )
    }

    /// The Jess name of the operation, as errors print it: `+`, `dot`, `sample`.
    #[must_use]
    pub fn operation(&self) -> &'static str {
        match self {
            Expression::Literal(_) => "literal",
            Expression::Parameter(_) => "parameter",
            Expression::Attribute(_) => "attribute",
            Expression::Varying(_) => "varying",
            Expression::Semantic(_) => "semantic",
            Expression::Sample { .. } => "sample",
            Expression::Add(..) => "+",
            Expression::Subtract(..) => "-",
            Expression::Multiply(..) => "*",
            Expression::Divide(..) => "/",
            Expression::Min(..) => "min",
            Expression::Max(..) => "max",
            Expression::Clamp(..) => "clamp",
            Expression::Dot(..) => "dot",
            Expression::Cross(..) => "cross",
            Expression::Normalize(..) => "normalize",
            Expression::Length(..) => "length",
            Expression::Floor(..) => "floor",
            Expression::Mix(..) => "mix",
            Expression::Construct { target, .. } => target.name(),
            Expression::Swizzle { .. } => "swizzle",
        }
    }
}
