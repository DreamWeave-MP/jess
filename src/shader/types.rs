//! The value types a shader expression can have.

use std::fmt;

/// What a value's components are made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scalar {
    /// A boolean.
    Bool,
    /// A signed 32-bit integer.
    Int,
    /// An unsigned 32-bit integer.
    UInt,
    /// A 32-bit float.
    Float,
}

/// Every type a shader value can have: scalars, vectors of two to four floats, signed integers, or
/// unsigned integers, and float matrices.
///
/// Structs, arrays, textures, and samplers are not values. A program declares them in its
/// interface and reads them through [`Place`](crate::shader::Place)s and texture operations, which
/// always produce one of these.
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
    /// Two signed integers.
    Vec2i,
    /// Three signed integers.
    Vec3i,
    /// Four signed integers.
    Vec4i,
    /// Two unsigned integers.
    Vec2u,
    /// Three unsigned integers.
    Vec3u,
    /// Four unsigned integers.
    Vec4u,
    /// A 3×3 float matrix, column-major.
    Mat3f,
    /// A 4×4 float matrix, column-major.
    Mat4f,
}

impl Type {
    /// The scalar of `scalar` (`size` 1) or the vector of `size` of them (2 to 4); `None` for any
    /// other size, and for vectors of `Bool`, which nothing needs.
    #[must_use]
    pub fn vector(scalar: Scalar, size: u8) -> Option<Type> {
        let ty = match (scalar, size) {
            (Scalar::Bool, 1) => Type::Bool,
            (Scalar::Int, 1) => Type::Int,
            (Scalar::UInt, 1) => Type::UInt,
            (Scalar::Float, 1) => Type::Float,
            (Scalar::Float, 2) => Type::Vec2f,
            (Scalar::Float, 3) => Type::Vec3f,
            (Scalar::Float, 4) => Type::Vec4f,
            (Scalar::Int, 2) => Type::Vec2i,
            (Scalar::Int, 3) => Type::Vec3i,
            (Scalar::Int, 4) => Type::Vec4i,
            (Scalar::UInt, 2) => Type::Vec2u,
            (Scalar::UInt, 3) => Type::Vec3u,
            (Scalar::UInt, 4) => Type::Vec4u,
            _ => return None,
        };
        Some(ty)
    }

    /// The vector with `count` float components: `Float` for 1, `Vec2f`..`Vec4f` for 2..4.
    #[must_use]
    pub fn float_vector(count: usize) -> Option<Type> {
        Type::vector(Scalar::Float, u8::try_from(count).ok()?)
    }

    /// What a scalar, vector, or matrix is made of; a matrix is made of floats.
    #[must_use]
    pub fn scalar(self) -> Scalar {
        match self {
            Type::Bool => Scalar::Bool,
            Type::Int | Type::Vec2i | Type::Vec3i | Type::Vec4i => Scalar::Int,
            Type::UInt | Type::Vec2u | Type::Vec3u | Type::Vec4u => Scalar::UInt,
            Type::Float | Type::Vec2f | Type::Vec3f | Type::Vec4f | Type::Mat3f | Type::Mat4f => Scalar::Float,
        }
    }

    /// How many components a scalar (1) or vector (2 to 4) has; 0 for a matrix.
    #[must_use]
    pub fn components(self) -> u8 {
        match self {
            Type::Bool | Type::Int | Type::UInt | Type::Float => 1,
            Type::Vec2f | Type::Vec2i | Type::Vec2u => 2,
            Type::Vec3f | Type::Vec3i | Type::Vec3u => 3,
            Type::Vec4f | Type::Vec4i | Type::Vec4u => 4,
            Type::Mat3f | Type::Mat4f => 0,
        }
    }

    /// A vector's component type, the scalar it is made of; `None` for anything else.
    #[must_use]
    pub fn component(self) -> Option<Type> {
        if !self.is_vector() {
            return None;
        }
        Type::vector(self.scalar(), 1)
    }

    /// A matrix's column type; `None` for anything else.
    #[must_use]
    pub fn column(self) -> Option<Type> {
        match self {
            Type::Mat3f => Some(Type::Vec3f),
            Type::Mat4f => Some(Type::Vec4f),
            _ => None,
        }
    }

    /// How many float components a `Float` or float vector has; 0 for every other type.
    #[must_use]
    pub fn float_components(self) -> u8 {
        if self.scalar() == Scalar::Float {
            self.components()
        } else {
            0
        }
    }

    /// Whether this is `Bool`, `Int`, `UInt`, or `Float`.
    #[must_use]
    pub fn is_scalar(self) -> bool {
        self.components() == 1
    }

    /// Whether this is a vector of any scalar.
    #[must_use]
    pub fn is_vector(self) -> bool {
        self.components() > 1
    }

    /// Whether this is `Vec2f`, `Vec3f`, or `Vec4f`.
    #[must_use]
    pub fn is_float_vector(self) -> bool {
        self.float_components() > 1
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

    /// Whether this is a numeric scalar or a vector: what arithmetic works on per component.
    #[must_use]
    pub fn is_numeric(self) -> bool {
        self.is_numeric_scalar() || self.is_vector()
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
            Type::Vec2i => "Vec2i",
            Type::Vec3i => "Vec3i",
            Type::Vec4i => "Vec4i",
            Type::Vec2u => "Vec2u",
            Type::Vec3u => "Vec3u",
            Type::Vec4u => "Vec4u",
            Type::Mat3f => "Mat3f",
            Type::Mat4f => "Mat4f",
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
