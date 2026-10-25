//! Texture kinds: 2D, layered, cube, and 3D textures, each sampled at its own coordinates and
//! declared with the WGSL type naga expects of it.

use super::common::{assert_valid_wgsl, read};

use jess::shader::wgsl;
use jess::shader::{Frequency, ProgramBuilder, Rate, ResourceKind, Swizzle, Type, ValidationErrorKind};

#[test]
fn each_kind_samples_at_its_coordinates() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    site.set_position(position).unwrap();
    let uvw = site.swizzle(position, Swizzle::parse("xyz").unwrap()).unwrap();
    let uvw = site.pass("uvw", uvw).unwrap();
    let uv = site.swizzle(uvw, Swizzle::parse("xy").unwrap()).unwrap();
    let layer = read(&mut site, Frequency::Draw, "layer", Type::UInt);

    let sampler = site.resource(Frequency::Pass, "linear", ResourceKind::Sampler).unwrap();
    let mut texels = Vec::new();
    for (name, kind, coordinates, layer) in [
        ("albedo", ResourceKind::Texture2D, uv, None),
        ("decals", ResourceKind::Texture2DArray, uv, Some(layer)),
        ("sky", ResourceKind::TextureCube, uvw, None),
        ("fog", ResourceKind::Texture3D, uvw, None),
    ] {
        let texture = site.resource(Frequency::Material, name, kind).unwrap();
        let texel = site.sample(texture, sampler, coordinates, layer).unwrap();
        assert_eq!(texel.ty(), Type::Vec4f);
        texels.push(texel);
    }
    let mut color = texels[0];
    for texel in &texels[1..] {
        color = site.add(color, *texel).unwrap();
    }
    site.output(color).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "var t_material_albedo: texture_2d<f32>;",
        "var t_material_decals: texture_2d_array<f32>;",
        "var t_material_sky: texture_cube<f32>;",
        "var t_material_fog: texture_3d<f32>;",
        "textureSample(t_material_decals, s_pass_linear, e0, u_draw.f_layer)",
        "textureSample(t_material_sky, s_pass_linear, input.v_uvw)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn a_sample_takes_what_its_texture_is_sampled_with() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let uv = read(&mut site, Frequency::Material, "uv", Type::Vec2f);
    let direction = read(&mut site, Frequency::Material, "direction", Type::Vec3f);
    let depth = read(&mut site, Frequency::Material, "depth", Type::Float);
    let layer = read(&mut site, Frequency::Material, "layer", Type::Int);
    let sampler = site.resource(Frequency::Pass, "linear", ResourceKind::Sampler).unwrap();
    let flat = site.resource(Frequency::Material, "flat", ResourceKind::Texture2D).unwrap();
    let decals = site
        .resource(Frequency::Material, "decals", ResourceKind::Texture2DArray)
        .unwrap();
    let sky = site.resource(Frequency::Material, "sky", ResourceKind::TextureCube).unwrap();

    let refusals = [
        (
            site.sample(sky, sampler, uv, None),
            "sample(TextureCube, Sampler, Vec2f) is invalid: expected sample(TextureCube, Sampler, Vec3f)",
        ),
        (
            site.sample(decals, sampler, uv, None),
            "sample(Texture2DArray, Sampler, Vec2f) is invalid: expected sample(Texture2DArray, Sampler, Vec2f, \
             an Int or UInt layer)",
        ),
        (
            site.sample(decals, sampler, uv, Some(depth)),
            "sample(Texture2DArray, Sampler, Vec2f, Float) is invalid: expected sample(Texture2DArray, Sampler, \
             Vec2f, an Int or UInt layer)",
        ),
        (
            site.sample(flat, sampler, uv, Some(layer)),
            "sample(Texture2D, Sampler, Vec2f, Int) is invalid: expected sample(Texture2D, Sampler, Vec2f)",
        ),
        (
            site.sample(flat, sampler, direction, None),
            "sample(Texture2D, Sampler, Vec3f) is invalid: expected sample(Texture2D, Sampler, Vec2f)",
        ),
    ];
    for (result, message) in refusals {
        let error = result.unwrap_err();
        assert!(matches!(error.kind, ValidationErrorKind::InvalidSample { .. }), "{error:?}");
        assert_eq!(error.kind.to_string(), message);
    }
}
