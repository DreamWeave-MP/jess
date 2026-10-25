//! Fragment outputs: one per color location, each a numeric scalar or vector, lowered at its own
//! type for the target behind it.

use super::common::{assert_valid_wgsl, read};

use jess::shader::wgsl;
use jess::shader::{Builtin, Frequency, ProgramBuilder, Rate, Swizzle, Type};

#[test]
fn outputs_lower_at_their_own_types() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    site.set_position(position).unwrap();
    let id = read(&mut site, Frequency::Draw, "id", Type::UInt);
    let one = site.float(1.0).unwrap();
    let color = site.construct(Type::Vec4f, &[one]).unwrap();
    let pixel = site.builtin(Builtin::Position);
    let depth = site.swizzle(pixel, Swizzle::parse("z").unwrap()).unwrap();
    let cell = site.swizzle(pixel, Swizzle::parse("xy").unwrap()).unwrap();
    let cell = site.convert(Type::Vec2i, cell).unwrap();
    for (expected, output) in [(0, color), (1, id), (2, depth), (3, cell)] {
        assert_eq!(site.output(output).unwrap(), expected);
    }

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    assert_eq!(module.fragment_outputs, 4);
    let expected = "struct FragmentOutput {
    @location(0) color0: vec4<f32>,
    @location(1) color1: u32,
    @location(2) color2: f32,
    @location(3) color3: vec2<i32>,
}";
    assert!(module.source.contains(expected), "no {expected} in:\n{}", module.source);
}
