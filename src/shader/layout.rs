//! Where everything a program declares lives: struct and uniform block layouts, vertex buffer
//! layouts, and bindings. None of it is the author's to choose.
//!
//! # Buffers
//!
//! The rules are WebGPU's for memory the host shares with the GPU, with one Jess rule on top: a
//! member of struct or array type is aligned to a multiple of 16 bytes, and a struct member's slot
//! is rounded up to a multiple of 16, so every struct is valid in a uniform block as well as
//! anywhere else and one layout serves both. Members are laid out by alignment, largest first,
//! then in declaration order, so no member pads the one after it, and a struct's size is a
//! multiple of its alignment. An array's stride is its element's size rounded up to the
//! element's alignment.
//!
//! # Bindings
//!
//! Each [`Frequency`] is one bind group ([`Frequency::group`]). Its uniform members, if it has
//! any, are one uniform block at binding 0; its resources follow, from binding 1 (from 0 when it
//! has no uniform members), in declaration order.
//!
//! # Vertex buffers
//!
//! Per-vertex attributes are buffer 0 and per-instance attributes buffer 1, each packed in
//! declaration order with no padding. Locations are numbered across both: every per-vertex
//! attribute's, then every per-instance attribute's. A matrix takes one location per column.

use crate::shader::interface::{AttributeId, DataType, Element, Frequency, Rate, ResourceId, StructId};
use crate::shader::program::Program;
use crate::shader::types::Type;

/// An alignment and size, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Extent {
    /// The alignment: a power of two.
    pub align: u32,
    /// The size.
    pub size: u32,
}

/// A value type's alignment and size in a buffer; `None` for `Bool` and resources, which no
/// buffer holds.
#[must_use]
pub fn value_extent(ty: Type) -> Option<Extent> {
    let (align, size) = match ty {
        Type::Int | Type::UInt | Type::Float => (4, 4),
        Type::Vec2f | Type::Vec2i | Type::Vec2u => (8, 8),
        Type::Vec3f | Type::Vec3i | Type::Vec3u => (16, 12),
        Type::Vec4f | Type::Vec4i | Type::Vec4u => (16, 16),
        Type::Mat3f => (16, 48),
        Type::Mat4f => (16, 64),
        Type::Bool | Type::Texture2D | Type::Sampler => return None,
    };
    Some(Extent { align, size })
}

/// One struct field's or uniform member's place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MemberLayout {
    /// Its byte offset from the start of the struct.
    pub offset: u32,
    /// The bytes it occupies, Jess's padding included: the next member starts no earlier than
    /// `offset + size`.
    pub size: u32,
    /// Its alignment, Jess's rule included.
    pub align: u32,
    /// For an array, the bytes from one element to the next.
    pub stride: Option<u32>,
}

/// A struct's layout, or a uniform block's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructLayout {
    /// The struct's alignment.
    pub align: u32,
    /// Its size, a multiple of its alignment.
    pub size: u32,
    /// Each member's place, by declaration position.
    pub members: Vec<MemberLayout>,
    /// The members' declaration positions in memory order.
    pub order: Vec<u32>,
}

impl StructLayout {
    /// The layout of members of `types`, in declaration order, given the layouts of the structs
    /// they may name.
    fn of(types: impl Iterator<Item = DataType>, structs: &[StructLayout]) -> StructLayout {
        let members: Vec<(Extent, Option<u32>)> = types.map(|ty| member_extent(ty, structs)).collect();
        let mut order: Vec<u32> = (0..).take(members.len()).collect();
        // Stable, so equal alignments keep declaration order.
        order.sort_by_key(|position| std::cmp::Reverse(members[*position as usize].0.align));

        let empty = MemberLayout {
            offset: 0,
            size: 0,
            align: 1,
            stride: None,
        };
        let mut layouts = vec![empty; members.len()];
        let (mut end, mut align) = (0u32, 1u32);
        for &position in &order {
            let (extent, stride) = members[position as usize];
            let offset = end.next_multiple_of(extent.align);
            layouts[position as usize] = MemberLayout {
                offset,
                size: extent.size,
                align: extent.align,
                stride,
            };
            end = offset + extent.size;
            align = align.max(extent.align);
        }
        StructLayout {
            align,
            size: end.next_multiple_of(align),
            members: layouts,
            order,
        }
    }
}

/// An element's natural extent; empty for anything validation rejects.
fn element_extent(element: Element, structs: &[StructLayout]) -> Extent {
    let empty = Extent { align: 1, size: 0 };
    match element {
        Element::Value(ty) => value_extent(ty).unwrap_or(empty),
        Element::Struct(id) => structs.get(id.index()).map_or(empty, |layout| Extent {
            align: layout.align,
            size: layout.size,
        }),
    }
}

/// The bytes from one element of an array to the next.
fn element_stride(element: Element, structs: &[StructLayout]) -> u32 {
    let extent = element_extent(element, structs);
    extent.size.next_multiple_of(extent.align)
}

/// A member's extent with Jess's rule applied, and its stride if it is an array.
fn member_extent(ty: DataType, structs: &[StructLayout]) -> (Extent, Option<u32>) {
    match ty {
        DataType::Value(ty) => (value_extent(ty).unwrap_or(Extent { align: 1, size: 0 }), None),
        DataType::Struct(id) => {
            let extent = element_extent(Element::Struct(id), structs);
            let padded = Extent {
                align: extent.align.next_multiple_of(16),
                size: extent.size.next_multiple_of(16),
            };
            (padded, None)
        }
        DataType::Array { element, length } => {
            let stride = element_stride(element, structs);
            let extent = Extent {
                align: element_extent(element, structs).align.next_multiple_of(16),
                size: stride.saturating_mul(length),
            };
            (extent, Some(stride))
        }
    }
}

/// One vertex attribute's place in its buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AttributeLayout {
    /// The attribute.
    pub attribute: AttributeId,
    /// Its type.
    pub ty: Type,
    /// Its byte offset in each vertex's or instance's record.
    pub offset: u32,
    /// Its first shader location.
    pub location: u32,
    /// How many locations it takes: a matrix's column count, else 1.
    pub locations: u32,
}

/// One vertex buffer's record layout.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VertexBufferLayout {
    /// The bytes from one record to the next; 0 when the buffer has no attributes.
    pub stride: u32,
    /// Its attributes, in declaration order.
    pub attributes: Vec<AttributeLayout>,
}

/// The bytes an attribute of `ty` takes in its record: its components, or a matrix's columns,
/// with no padding.
#[must_use]
pub fn attribute_size(ty: Type) -> u32 {
    if let Some(column) = ty.column() {
        let rows = u32::from(column.components());
        return rows * rows * 4;
    }
    u32::from(ty.components()) * 4
}

/// Where a resource is bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResourceBinding {
    /// The resource.
    pub resource: ResourceId,
    /// Its bind group: its frequency's.
    pub group: u32,
    /// Its binding in the group.
    pub binding: u32,
}

/// One frequency's bind group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupLayout {
    /// Who binds it.
    pub frequency: Frequency,
    /// The uniform block at binding 0, if the frequency has uniform members: its members are the
    /// frequency's, in declaration order.
    pub uniform: Option<StructLayout>,
    /// The frequency's resources, in declaration order.
    pub resources: Vec<ResourceBinding>,
}

/// Everything about where a program's interface lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramLayout {
    /// Every declared struct's layout, by declaration position.
    pub structs: Vec<StructLayout>,
    /// One bind group per frequency, in [`Frequency::ALL`] order.
    pub groups: [GroupLayout; 3],
    /// The per-vertex buffer (0) and per-instance buffer (1).
    pub vertex_buffers: [VertexBufferLayout; 2],
}

impl ProgramLayout {
    /// The layout of everything `program` declares. A declaration validation rejects (a `Bool`
    /// member, a struct naming one declared after it) lays out as empty, never as a panic.
    #[must_use]
    pub fn of(program: &Program) -> ProgramLayout {
        let (structs, attributes, uniforms, resources) =
            (&program.structs, &program.attributes, &program.uniforms, &program.resources);
        let mut struct_layouts: Vec<StructLayout> = Vec::with_capacity(structs.len());
        for declaration in structs {
            let layout = StructLayout::of(declaration.fields.iter().map(|field| field.ty), &struct_layouts);
            struct_layouts.push(layout);
        }

        let groups = Frequency::ALL.map(|frequency| {
            let members = uniforms.iter().filter(|uniform| uniform.frequency == frequency);
            let uniform = StructLayout::of(members.map(|uniform| uniform.ty), &struct_layouts);
            let uniform = (!uniform.members.is_empty()).then_some(uniform);
            let mut binding = u32::from(uniform.is_some());
            let mut bound = Vec::new();
            for (index, resource) in resources.iter().enumerate() {
                if resource.frequency != frequency {
                    continue;
                }
                bound.push(ResourceBinding {
                    resource: ResourceId::at(index),
                    group: frequency.group(),
                    binding,
                });
                binding += 1;
            }
            GroupLayout {
                frequency,
                uniform,
                resources: bound,
            }
        });

        let mut location = 0;
        let vertex_buffers = Rate::ALL.map(|rate| {
            let mut buffer = VertexBufferLayout::default();
            for (index, attribute) in attributes.iter().enumerate() {
                if attribute.rate != rate {
                    continue;
                }
                let locations = attribute.ty.column().map_or(1, |column| u32::from(column.components()));
                buffer.attributes.push(AttributeLayout {
                    attribute: AttributeId::at(index),
                    ty: attribute.ty,
                    offset: buffer.stride,
                    location,
                    locations,
                });
                buffer.stride += attribute_size(attribute.ty);
                location += locations;
            }
            buffer
        });

        ProgramLayout {
            structs: struct_layouts,
            groups,
            vertex_buffers,
        }
    }

    /// A struct's layout.
    #[must_use]
    pub fn structure(&self, id: StructId) -> Option<&StructLayout> {
        self.structs.get(id.index())
    }

    /// The bind group `frequency` has.
    #[must_use]
    pub fn group(&self, frequency: Frequency) -> &GroupLayout {
        &self.groups[frequency as usize]
    }

    /// Where `resource` is bound.
    #[must_use]
    pub fn binding(&self, resource: ResourceId) -> Option<ResourceBinding> {
        self.groups
            .iter()
            .flat_map(|group| &group.resources)
            .find(|binding| binding.resource == resource)
            .copied()
    }

    /// Where `attribute` is in its buffer.
    #[must_use]
    pub fn attribute(&self, attribute: AttributeId) -> Option<&AttributeLayout> {
        self.vertex_buffers
            .iter()
            .flat_map(|buffer| &buffer.attributes)
            .find(|layout| layout.attribute == attribute)
    }
}
