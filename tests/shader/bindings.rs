//! Bindings per frequency: structs and arrays in uniform blocks, read through places, and
//! textures and samplers at the bindings the layout gives them, every one checked by naga.

use super::common::{assert_valid_wgsl, member};

use jess::shader::wgsl;
use jess::shader::{
    DataType, Element, Field, Frequency, PlaceRoot, ProgramBuilder, Rate, ResourceKind, Step, Type, ValidationErrorKind,
};

#[test]
fn structs_and_arrays_lower_with_the_layout_jess_reports() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let value = DataType::Value;
    let light = site
        .structure(
            "Light",
            vec![
                Field::new("color", value(Type::Vec3f)),
                Field::new("direction", value(Type::Vec3f)),
            ],
        )
        .unwrap();
    // Eight bytes alone: in a uniform block it needs the 16-byte slot jess gives it.
    let fade = site.structure("Fade", vec![Field::new("range", value(Type::Vec2f))]).unwrap();
    let lights = DataType::Array {
        element: Element::Struct(light),
        length: 4,
    };
    site.uniform(Frequency::Pass, "lights", lights).unwrap();
    site.uniform(Frequency::Pass, "fade", DataType::Struct(fade)).unwrap();
    site.uniform(Frequency::Pass, "exposure", value(Type::Float)).unwrap();
    let shadow = site.resource(Frequency::Pass, "shadow", ResourceKind::Texture2D).unwrap();
    let shadow_sampler = site.resource(Frequency::Pass, "shadow", ResourceKind::Sampler).unwrap();
    let diffuse = site
        .resource(Frequency::Material, "diffuse", ResourceKind::Texture2D)
        .unwrap();
    let diffuse_sampler = site.resource(Frequency::Material, "diffuse", ResourceKind::Sampler).unwrap();

    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let which = site.attribute(Rate::Vertex, "light", Type::UInt).unwrap();
    let which = site.read_attribute(which);
    site.set_position(position).unwrap();
    let lights = member(Frequency::Pass, 0);
    let color = lights
        .then(Step::Index(which.expression()))
        .unwrap()
        .then(Step::Field(0))
        .unwrap();
    let color = site.load(color, &[which]).unwrap();
    assert_eq!(color.ty(), Type::Vec3f);
    let color = site.pass("color", color).unwrap();

    let range = member(Frequency::Pass, 1).then(Step::Field(0)).unwrap();
    let range = site.load(range, &[]).unwrap();
    let uv = site.swizzle(range, jess::shader::Swizzle::parse("xy").unwrap()).unwrap();
    let lit = site.sample(diffuse, diffuse_sampler, uv, None).unwrap();
    let shade = site.sample(shadow, shadow_sampler, uv, None).unwrap();
    let exposure = site.load(member(Frequency::Pass, 2), &[]).unwrap();
    let one = site.float(1.0).unwrap();
    let tinted = site.construct(Type::Vec4f, &[color, one]).unwrap();
    let lit = site.multiply(lit, tinted).unwrap();
    let lit = site.multiply(lit, shade).unwrap();
    let lit = site.multiply(lit, exposure).unwrap();
    site.output(lit).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "struct S0_Light {",
        "@align(16) @size(16) f_fade: S1_Fade,",
        "f_lights: array<S0_Light, 4>,",
        "@group(0) @binding(1) var t_pass_shadow: texture_2d<f32>;",
        "@group(0) @binding(2) var s_pass_shadow: sampler;",
        "@group(1) @binding(0) var t_material_diffuse: texture_2d<f32>;",
        "let e0 = u_pass.f_lights[input.a_light].f_color;",
        "textureSample(t_material_diffuse, s_material_diffuse, e0)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
    let block = module.layout.group(Frequency::Pass).uniform.as_ref().unwrap();
    assert_eq!(
        block.size, 160,
        "four 32-byte lights, the fade's 16-byte slot, and the exposure, rounded up to 16"
    );
}

#[test]
fn a_place_is_checked_where_it_is_read() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let weights = DataType::Array {
        element: Element::Value(Type::Vec4f),
        length: 4,
    };
    site.uniform(Frequency::Draw, "weights", weights).unwrap();
    let four = site.uint(4);
    let past = member(Frequency::Draw, 0).then(Step::Index(four.expression())).unwrap();
    let error = site.load(past, &[four]).unwrap_err();
    assert!(matches!(error.kind, ValidationErrorKind::InvalidPlace { .. }), "{error:?}");
    assert_eq!(error.to_string(), "a load reads element 4 of an array of 4");

    let whole = site.load(member(Frequency::Draw, 0), &[]).unwrap_err();
    assert_eq!(
        whole.to_string(),
        "a load ends at a struct or an array, not a value: read one of its fields or elements"
    );
    assert_eq!(member(Frequency::Draw, 0).root, PlaceRoot::Uniform(Frequency::Draw));
}
