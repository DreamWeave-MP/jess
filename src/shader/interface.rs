//! What a program declares it reads: vertex attributes, uniform members, textures, samplers, and
//! the structs they are made of.
//!
//! A program names its inputs by what they are and how often they change. Jess decides where they
//! live: every layout, every binding, every attribute location (see [`layout`](crate::shader::layout)).

use std::fmt;

use crate::shader::expression::SourceOrigin;
use crate::shader::types::Type;

arena_id!(
    /// A struct, by its position among a program's declared structs.
    StructId,
    "struct's position in declaration order"
);
arena_id!(
    /// A vertex attribute, by its position among a program's declared attributes of both rates.
    AttributeId,
    "attribute's position in declaration order, across both rates"
);
arena_id!(
    /// A uniform member, by its position among a program's declared members of every frequency.
    UniformId,
    "uniform member's position in declaration order, across every frequency"
);
arena_id!(
    /// A texture or sampler, by its position among a program's declared resources of every
    /// frequency.
    ResourceId,
    "resource's position in declaration order, across every frequency"
);

/// How often a uniform member or resource changes, which says who binds it.
///
/// Each frequency is one bind group: the pass binds what every draw in it shares (a camera, a
/// shadow map, the previous pass's output), the material what one program's draws share (a tint,
/// a texture), and the draw what one draw alone has (its own transform block).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Frequency {
    /// Bound once per pass.
    Pass,
    /// Bound with the material.
    Material,
    /// Bound per draw.
    Draw,
}

impl Frequency {
    /// Every frequency, in bind group order.
    pub const ALL: [Frequency; 3] = [Frequency::Pass, Frequency::Material, Frequency::Draw];

    /// The frequency's name: `pass`, `material`, or `draw`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Frequency::Pass => "pass",
            Frequency::Material => "material",
            Frequency::Draw => "draw",
        }
    }

    /// Its bind group.
    #[must_use]
    pub fn group(self) -> u32 {
        self as u32
    }
}

impl fmt::Display for Frequency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

/// How often a vertex attribute advances: per vertex, or per instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rate {
    /// One value per vertex.
    Vertex,
    /// One value per instance.
    Instance,
}

impl Rate {
    /// Both rates, in vertex buffer order.
    pub const ALL: [Rate; 2] = [Rate::Vertex, Rate::Instance];

    /// The rate's name, as errors print it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Rate::Vertex => "attribute",
            Rate::Instance => "instance attribute",
        }
    }
}

/// What an array holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(variant_size_differences)] // an id beside a one-byte type: a few bytes either way
pub enum Element {
    /// A value.
    Value(Type),
    /// A struct.
    Struct(StructId),
}

impl From<Element> for DataType {
    fn from(element: Element) -> DataType {
        match element {
            Element::Value(ty) => DataType::Value(ty),
            Element::Struct(id) => DataType::Struct(id),
        }
    }
}

/// The type of something a buffer holds: a struct field or a uniform member.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(variant_size_differences)] // an id beside a one-byte type: a few bytes either way
pub enum DataType {
    /// A value.
    Value(Type),
    /// A struct.
    Struct(StructId),
    /// A fixed number of elements.
    Array {
        /// What each element is.
        element: Element,
        /// How many there are; at least 1.
        length: u32,
    },
}

/// One field of a struct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The field's name.
    pub name: String,
    /// Its type.
    pub ty: DataType,
}

impl Field {
    /// A field `name` of `ty`.
    #[must_use]
    pub fn new(name: &str, ty: DataType) -> Field {
        Field {
            name: name.to_owned(),
            ty,
        }
    }
}

/// A struct: named fields in a buffer. Its layout is Jess's; the order the author gave the fields
/// in is the order reflection and errors list them in, not their order in memory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructDecl {
    /// The struct's name, as errors print it.
    pub name: String,
    /// Its fields, in declaration order. A field may only name a struct declared before this one.
    pub fields: Vec<Field>,
    /// Where the author declared it, if known.
    pub origin: Option<SourceOrigin>,
}

impl StructDecl {
    /// A struct `name` of `fields`, with no origin.
    #[must_use]
    pub fn new(name: &str, fields: Vec<Field>) -> StructDecl {
        StructDecl {
            name: name.to_owned(),
            fields,
            origin: None,
        }
    }
}

/// A vertex attribute: one value per vertex or per instance, read from a vertex buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    /// The attribute's name.
    pub name: String,
    /// Per vertex or per instance.
    pub rate: Rate,
    /// Its type: an `Int`, `UInt`, or `Float`, a vector of one, or a matrix, which takes one
    /// location per column.
    pub ty: Type,
    /// Where the author declared it, if known.
    pub origin: Option<SourceOrigin>,
}

impl Attribute {
    /// An attribute `name` of `ty` at `rate`, with no origin.
    #[must_use]
    pub fn new(rate: Rate, name: &str, ty: Type) -> Attribute {
        Attribute {
            name: name.to_owned(),
            rate,
            ty,
            origin: None,
        }
    }
}

/// A member of a frequency's uniform block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Uniform {
    /// The member's name.
    pub name: String,
    /// Who binds it.
    pub frequency: Frequency,
    /// Its type.
    pub ty: DataType,
    /// Where the author declared it, if known.
    pub origin: Option<SourceOrigin>,
}

impl Uniform {
    /// A member `name` of `ty` that `frequency` binds, with no origin.
    #[must_use]
    pub fn new(frequency: Frequency, name: &str, ty: DataType) -> Uniform {
        Uniform {
            name: name.to_owned(),
            frequency,
            ty,
            origin: None,
        }
    }
}

/// What a resource is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// A 2D texture of filterable floats, sampled to a `Vec4f`.
    Texture2D,
    /// A filtering sampler.
    Sampler,
}

impl ResourceKind {
    /// The kind's name, as errors print it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            ResourceKind::Texture2D => "Texture2D",
            ResourceKind::Sampler => "Sampler",
        }
    }
}

/// A texture or sampler some frequency binds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// The resource's name. A texture and the sampler declared with it may share one.
    pub name: String,
    /// Who binds it.
    pub frequency: Frequency,
    /// What it is.
    pub kind: ResourceKind,
    /// Where the author declared it, if known.
    pub origin: Option<SourceOrigin>,
}

impl Resource {
    /// A resource `name` of `kind` that `frequency` binds, with no origin.
    #[must_use]
    pub fn new(frequency: Frequency, name: &str, kind: ResourceKind) -> Resource {
        Resource {
            name: name.to_owned(),
            frequency,
            kind,
            origin: None,
        }
    }
}
