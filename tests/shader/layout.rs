//! Jess lays out everything a program declares: struct and uniform block members by alignment,
//! struct and array members padded to 16 bytes so one layout is valid in every buffer, bindings by
//! frequency, and vertex attributes packed per rate.

use jess::shader::layout::{MemberLayout, ResourceBinding};
use jess::shader::{
    Attribute, DataType, Element, Field, Frequency, Program, Rate, Resource, ResourceKind, StructDecl, Type, Uniform,
};

fn member(offset: u32, size: u32, align: u32) -> MemberLayout {
    MemberLayout {
        offset,
        size,
        align,
        stride: None,
    }
}

fn light() -> StructDecl {
    StructDecl::new(
        "Light",
        vec![
            Field::new("intensity", DataType::Value(Type::Float)),
            Field::new("color", DataType::Value(Type::Vec3f)),
            Field::new("direction", DataType::Value(Type::Vec3f)),
            Field::new("flags", DataType::Value(Type::UInt)),
        ],
    )
}

#[test]
fn members_lie_largest_alignment_first_so_none_pads_the_next() {
    let mut program = Program::new();
    let id = program.structure(light());
    let layout = program.layout();
    let light = layout.structure(id).unwrap();
    assert_eq!(light.order, [1, 2, 0, 3]);
    assert_eq!(
        light.members,
        [member(28, 4, 4), member(0, 12, 16), member(16, 12, 16), member(32, 4, 4)],
        "intensity fills the padding after direction"
    );
    assert_eq!((light.align, light.size), (16, 48));
}

#[test]
fn struct_and_array_members_take_whole_16_byte_slots() {
    let mut program = Program::new();
    let pair = program.structure(StructDecl::new(
        "Pair",
        vec![Field::new("value", DataType::Value(Type::Vec2f))],
    ));
    let light = program.structure(light());
    let path = program.structure(StructDecl::new(
        "Path",
        vec![
            Field::new(
                "points",
                DataType::Array {
                    element: Element::Struct(pair),
                    length: 3,
                },
            ),
            Field::new("length", DataType::Value(Type::Float)),
        ],
    ));
    let members = [
        ("exposure", DataType::Value(Type::Float)),
        ("light", DataType::Struct(light)),
        (
            "weights",
            DataType::Array {
                element: Element::Value(Type::Vec4f),
                length: 4,
            },
        ),
        ("pair", DataType::Struct(pair)),
        (
            "lights",
            DataType::Array {
                element: Element::Struct(light),
                length: 2,
            },
        ),
        ("rotation", DataType::Value(Type::Mat3f)),
    ];
    for (name, ty) in members {
        program.uniform(Uniform::new(Frequency::Pass, name, ty));
    }
    let layout = program.layout();
    let natural = layout.structure(pair).unwrap();
    assert_eq!(
        (natural.align, natural.size),
        (8, 8),
        "a struct alone keeps its natural layout"
    );
    let path = layout.structure(path).unwrap();
    let points = path.members[0];
    assert_eq!(
        (points.offset, points.size, points.align, points.stride),
        (0, 24, 16, Some(8))
    );
    assert_eq!((path.members[1].offset, path.size), (24, 32));

    let block = layout.group(Frequency::Pass).uniform.as_ref().unwrap();
    let placed: Vec<(u32, u32, u32, Option<u32>)> = block
        .members
        .iter()
        .map(|member| (member.offset, member.size, member.align, member.stride))
        .collect();
    assert_eq!(
        placed,
        [
            (272, 4, 4, None),
            (0, 48, 16, None),
            (48, 64, 16, Some(16)),
            (112, 16, 16, None),
            (128, 96, 16, Some(48)),
            (224, 48, 16, None),
        ]
    );
    assert_eq!((block.align, block.size), (16, 288));
}

#[test]
fn each_frequency_binds_its_block_then_its_resources() {
    let mut program = Program::new();
    program.uniform(Uniform::new(Frequency::Pass, "camera", DataType::Value(Type::Mat4f)));
    program.uniform(Uniform::new(Frequency::Material, "tint", DataType::Value(Type::Vec4f)));
    let declared = [
        (Frequency::Material, "diffuse", ResourceKind::Texture2D),
        (Frequency::Material, "diffuse", ResourceKind::Sampler),
        (Frequency::Pass, "shadow", ResourceKind::Texture2D),
        (Frequency::Draw, "detail", ResourceKind::Texture2D),
    ];
    let ids: Vec<_> = declared
        .into_iter()
        .map(|(frequency, name, kind)| program.resource(Resource::new(frequency, name, kind)))
        .collect();
    let layout = program.layout();
    let bound: Vec<ResourceBinding> = ids.iter().map(|id| layout.binding(*id).unwrap()).collect();
    let at = |index: usize, group: u32, binding: u32| ResourceBinding {
        resource: ids[index],
        group,
        binding,
    };
    assert_eq!(
        bound,
        [at(0, 1, 1), at(1, 1, 2), at(2, 0, 1), at(3, 2, 0)],
        "a frequency with no uniform members starts its resources at 0"
    );
    assert!(layout.group(Frequency::Draw).uniform.is_none());
    assert_eq!(layout.group(Frequency::Pass).uniform.as_ref().unwrap().size, 64);
}

#[test]
fn attributes_pack_per_rate_and_number_locations_across_both() {
    let mut program = Program::new();
    let declared = [
        (Rate::Vertex, "position", Type::Vec3f),
        (Rate::Vertex, "uv", Type::Vec2f),
        (Rate::Instance, "model", Type::Mat4f),
        (Rate::Instance, "tint", Type::Vec4f),
        (Rate::Vertex, "id", Type::UInt),
    ];
    let ids: Vec<_> = declared
        .into_iter()
        .map(|(rate, name, ty)| program.attribute(Attribute::new(rate, name, ty)))
        .collect();
    let layout = program.layout();
    let placed = |buffer: usize| -> Vec<(usize, u32, u32, u32)> {
        layout.vertex_buffers[buffer]
            .attributes
            .iter()
            .map(|attribute| {
                (
                    attribute.attribute.index(),
                    attribute.offset,
                    attribute.location,
                    attribute.locations,
                )
            })
            .collect()
    };
    assert_eq!(placed(0), [(0, 0, 0, 1), (1, 12, 1, 1), (4, 20, 2, 1)]);
    assert_eq!(layout.vertex_buffers[0].stride, 24);
    assert_eq!(
        placed(1),
        [(2, 0, 3, 4), (3, 64, 7, 1)],
        "a matrix takes a location per column"
    );
    assert_eq!(layout.vertex_buffers[1].stride, 80);
    assert_eq!(layout.attribute(ids[3]).unwrap().offset, 64);
}
