//! Conversions between scalars, and matrices built from their columns.

use super::common::{assert_valid_wgsl, read};

use jess::shader::wgsl;
use jess::shader::{Frequency, ProgramBuilder, Rate, Type, ValidationErrorKind};

#[test]
fn values_convert_between_scalars_and_matrices_build_from_columns() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec3f).unwrap();
    let position = site.read_attribute(position);
    let cell = site.attribute(Rate::Instance, "cell", Type::Vec3u).unwrap();
    let cell = site.read_attribute(cell);
    let index = site.attribute(Rate::Instance, "index", Type::UInt).unwrap();
    let index = site.read_attribute(index);
    let right = read(&mut site, Frequency::Draw, "right", Type::Vec4f);
    let up = read(&mut site, Frequency::Draw, "up", Type::Vec4f);
    let forward = read(&mut site, Frequency::Draw, "forward", Type::Vec4f);

    let offset = site.convert(Type::Vec3f, cell).unwrap();
    assert_eq!(offset.ty(), Type::Vec3f);
    let moved = site.add(position, offset).unwrap();
    let shade = site.convert(Type::Float, index).unwrap();
    let one = site.float(1.0).unwrap();
    let placed = site.construct(Type::Vec4f, &[moved, one]).unwrap();
    let zero = site.float(0.0).unwrap();
    let origin = site.construct(Type::Vec4f, &[zero, zero, zero, one]).unwrap();
    let basis = site.construct(Type::Mat4f, &[right, up, forward, origin]).unwrap();
    assert_eq!(basis.ty(), Type::Mat4f);
    let clip = site.multiply(basis, placed).unwrap();
    site.set_position(clip).unwrap();
    site.pass("shade", shade).unwrap();
    let signed = site.convert(Type::Vec3i, cell).unwrap();
    site.pass("signed", signed).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "vec3<f32>(input.i_cell)",
        "f32(input.i_index)",
        "vec3<i32>(input.i_cell)",
        "mat4x4<f32>(u_draw.f_right, u_draw.f_up, u_draw.f_forward, e4)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn a_conversion_keeps_the_size_and_a_matrix_takes_its_columns() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let cell = read(&mut site, Frequency::Material, "cell", Type::Vec3u);
    let column = read(&mut site, Frequency::Material, "column", Type::Vec3f);
    let basis = read(&mut site, Frequency::Material, "basis", Type::Mat3f);
    let half = site.float(0.5).unwrap();

    let refusals = [
        (
            site.convert(Type::Vec2f, cell).unwrap_err().kind,
            "Vec2f(Vec3u) is invalid: expected a scalar or vector of the same size",
        ),
        (
            site.convert(Type::Float, cell).unwrap_err().kind,
            "Float(Vec3u) is invalid: expected a scalar or vector of the same size",
        ),
        (
            site.convert(Type::Mat3f, basis).unwrap_err().kind,
            "Mat3f(Mat3f) is invalid: expected a scalar or vector of the same size",
        ),
        (
            site.construct(Type::Mat3f, &[column, column]).unwrap_err().kind,
            "Mat3f(Vec3f, Vec3f) is invalid: expected three Vec3f columns",
        ),
        (
            site.construct(Type::Mat4f, &[half]).unwrap_err().kind,
            "Mat4f(Float) is invalid: expected four Vec4f columns",
        ),
    ];
    for (kind, message) in refusals {
        assert!(matches!(kind, ValidationErrorKind::InvalidOperation { .. }), "{kind:?}");
        assert_eq!(kind.to_string(), message);
    }
}
