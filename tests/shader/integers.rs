//! Integer vectors: the same arithmetic, constructors, and swizzles as float vectors, a scalar of
//! the vector's own kind broadcast against it, and nothing that only makes sense for floats.

use super::common::assert_valid_wgsl;

use jess::shader::wgsl;
use jess::shader::{ProgramBuilder, Rate, Swizzle, Type, ValidationErrorKind};

#[test]
fn integer_vectors_compute_and_pass_flat() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let ids = site.attribute(Rate::Vertex, "ids", Type::Vec4u).unwrap();
    let ids = site.read_attribute(ids);
    let offset = site.parameter("offset", Type::Vec3i).unwrap();
    let scale = site.parameter("scale", Type::UInt).unwrap();
    site.set_position(position).unwrap();

    let one = site.int(1);
    let shifted = site.add(offset, one).unwrap();
    assert_eq!(shifted.ty(), Type::Vec3i);
    let (low, high) = (site.int(0), site.int(7));
    let cell = site.clamp(shifted, low, high).unwrap();
    site.pass("cell", cell).unwrap();

    let weight = site.dot(offset, offset).unwrap();
    assert_eq!(weight.ty(), Type::Int, "dot gives the component type");
    site.pass("weight", weight).unwrap();

    let pair = site.swizzle(ids, Swizzle::parse("zx").unwrap()).unwrap();
    assert_eq!(pair.ty(), Type::Vec2u);
    let scaled = site.multiply(pair, scale).unwrap();
    let largest = site.max(scaled, scale).unwrap();
    site.pass("pair", largest).unwrap();

    let packed = site.construct(Type::Vec4i, &[offset, one]).unwrap();
    site.pass("packed", packed).unwrap();
    let splat = site.construct(Type::Vec3u, &[scale]).unwrap();
    site.pass("splat", splat).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "@location(1) a_ids: vec4<u32>",
        "p_offset: vec3<i32>",
        "@interpolate(flat) v_cell: vec3<i32>",
        "clamp(e0, vec3<i32>(0i), vec3<i32>(7i))",
        "max(e4, vec2<u32>(parameters.p_scale))",
        "vec4<i32>(parameters.p_offset, 1i)",
        "vec3<u32>(parameters.p_scale)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
    let layout: Vec<(u32, u32)> = module
        .parameters
        .unwrap()
        .members
        .iter()
        .map(|member| (member.offset, member.size))
        .collect();
    assert_eq!(layout, [(0, 12), (12, 4)], "a UInt fits in a Vec3i's padding");
}

#[test]
fn float_only_operations_refuse_integers() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let cells = site.parameter("cells", Type::Vec3i).unwrap();
    let count = site.parameter("count", Type::UInt).unwrap();
    let half = site.float(0.5).unwrap();

    let refusals = [
        (
            site.normalize(cells).unwrap_err().kind,
            "normalize(Vec3i) is invalid: expected a float vector",
        ),
        (
            site.add(cells, half).unwrap_err().kind,
            "Vec3i + Float is invalid: expected two values of the same numeric type, a vector and a scalar of its \
             kind, or two matrices of the same size",
        ),
        (
            site.multiply(cells, count).unwrap_err().kind,
            "Vec3i * UInt is invalid: expected two values of the same numeric type, a vector and a scalar of its \
             kind, a matrix and a Float, a matrix and a vector of its size, or two matrices of the same size",
        ),
        (
            site.construct(Type::Vec2i, &[half, half]).unwrap_err().kind,
            "Vec2i(Float, Float) is invalid: expected Ints and Int vectors adding up to 2 components, or one Int",
        ),
        (
            site.construct(Type::Vec4u, &[cells, count]).unwrap_err().kind,
            "Vec4u(Vec3i, UInt) is invalid: expected UInts and UInt vectors adding up to 4 components, or one UInt",
        ),
        (
            site.length(cells).unwrap_err().kind,
            "length(Vec3i) is invalid: expected a Float or float vector",
        ),
    ];
    for (kind, message) in refusals {
        assert!(matches!(kind, ValidationErrorKind::InvalidOperation { .. }), "{kind:?}");
        assert_eq!(kind.to_string(), message);
    }

    let swizzle = site.swizzle(cells, Swizzle::parse("xyzw").unwrap()).unwrap_err();
    assert_eq!(
        swizzle.kind.to_string(),
        "Vec3i.xyzw is invalid: expected components a Vec3i has"
    );
}
