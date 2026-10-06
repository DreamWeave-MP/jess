//! The value types a shader expression can have.

use std::fmt;

/// Every type a shader expression, parameter, vertex input, or varying can have.
///
/// Vectors and matrices are `f32` only: nothing the renderer draws needs integer vectors yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    /// A boolean.
    Bool,
    /// A signed 32-bit integer.
    Int,
    /// An unsigned 32-bit integer.
    UInt,
    /// A 32-bit float.
    Float,
    /// Two floats.
    Vec2f,
    /// Three floats.
    Vec3f,
    /// Four floats.
    Vec4f,
    /// A 3×3 float matrix, column-major.
    Mat3f,
    /// A 4×4 float matrix, column-major.
    Mat4f,
    /// A sampled 2D float texture. Only a parameter can have this type.
    Texture2D,
    /// A filtering sampler. Only a parameter can have this type.
    Sampler,
}

impl Type {
    /// The vector with `count` float components: `Float` for 1, `Vec2f`..`Vec4f` for 2..4.
    #[must_use]
    pub fn float_vector(count: usize) -> Option<Type> {
        match count {
            1 => Some(Type::Float),
            2 => Some(Type::Vec2f),
            3 => Some(Type::Vec3f),
            4 => Some(Type::Vec4f),
            _ => None,
        }
    }

    /// How many float components a `Float` or float vector has; 0 for every other type.
    #[must_use]
    pub fn float_components(self) -> u8 {
        match self {
            Type::Float => 1,
            Type::Vec2f => 2,
            Type::Vec3f => 3,
            Type::Vec4f => 4,
            _ => 0,
        }
    }

    /// Whether this is `Vec2f`, `Vec3f`, or `Vec4f`.
    #[must_use]
    pub fn is_vector(self) -> bool {
        matches!(self, Type::Vec2f | Type::Vec3f | Type::Vec4f)
    }

    /// Whether this is `Mat3f` or `Mat4f`.
    #[must_use]
    pub fn is_matrix(self) -> bool {
        matches!(self, Type::Mat3f | Type::Mat4f)
    }

    /// Whether this is `Int`, `UInt`, or `Float`.
    #[must_use]
    pub fn is_numeric_scalar(self) -> bool {
        matches!(self, Type::Int | Type::UInt | Type::Float)
    }

    /// Whether this is `Texture2D` or `Sampler`: a bound resource, not a value.
    #[must_use]
    pub fn is_resource(self) -> bool {
        matches!(self, Type::Texture2D | Type::Sampler)
    }

    /// The Jess name of the type, as errors print it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Type::Bool => "Bool",
            Type::Int => "Int",
            Type::UInt => "UInt",
            Type::Float => "Float",
            Type::Vec2f => "Vec2f",
            Type::Vec3f => "Vec3f",
            Type::Vec4f => "Vec4f",
            Type::Mat3f => "Mat3f",
            Type::Mat4f => "Mat4f",
            Type::Texture2D => "Texture2D",
            Type::Sampler => "Sampler",
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
