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
        assert!(matches!(error.kind, ValidationErrorKind::InvalidTexture { .. }), "{error:?}");
        assert_eq!(error.kind.to_string(), message);
    }
}

#[test]
fn textures_are_read_at_levels_by_texel_and_asked_their_size() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let uv = site.attribute(Rate::Vertex, "uv", Type::Vec2f).unwrap();
    let uv = site.read_attribute(uv);
    let sampler = site.resource(Frequency::Pass, "linear", ResourceKind::Sampler).unwrap();
    let height = site.resource(Frequency::Material, "height", ResourceKind::Texture2D).unwrap();
    let ids = site.resource(Frequency::Material, "ids", ResourceKind::Texture2Du).unwrap();
    let offsets = site
        .resource(Frequency::Material, "offsets", ResourceKind::Texture2Di)
        .unwrap();
    let decals = site
        .resource(Frequency::Material, "decals", ResourceKind::Texture2DArray)
        .unwrap();
    let fog = site.resource(Frequency::Material, "fog", ResourceKind::Texture3D).unwrap();

    // The vertex stage has no derivatives, so it samples at a level it names.
    let zero = site.float(0.0).unwrap();
    let lifted = site.sample_level(height, sampler, uv, None, zero).unwrap();
    let lift = site.swizzle(lifted, Swizzle::parse("x").unwrap()).unwrap();
    let one = site.float(1.0).unwrap();
    let position = site.construct(Type::Vec4f, &[uv, lift, one]).unwrap();
    site.set_position(position).unwrap();
    let uv = site.pass("uv", uv).unwrap();

    let extent = site.dimensions(height, None).unwrap();
    assert_eq!(extent.ty(), Type::Vec2u);
    let volume = site.dimensions(fog, None).unwrap();
    assert_eq!(volume.ty(), Type::Vec3u);
    let levels = site.levels(height).unwrap();
    let layers = site.layers(decals).unwrap();
    assert_eq!((levels.ty(), layers.ty()), (Type::UInt, Type::UInt));

    let texel = site.convert(Type::Vec2f, extent).unwrap();
    let texel = site.multiply(uv, texel).unwrap();
    let texel = site.convert(Type::Vec2u, texel).unwrap();
    let base = site.uint(0);
    let id = site.load_texel(ids, texel, None, base).unwrap();
    assert_eq!(id.ty(), Type::Vec4u);
    let offset = site.load_texel(offsets, texel, None, base).unwrap();
    assert_eq!(offset.ty(), Type::Vec4i);
    let last = site.uint(1);
    let last = site.binary(jess::shader::BinaryOperator::Subtract, layers, last).unwrap();
    let decal = site.load_texel(decals, texel, Some(last), base).unwrap();
    let bias = site.float(-0.5).unwrap();
    let sharp = site.sample_bias(height, sampler, uv, None, bias).unwrap();

    let id = site.convert(Type::Vec4f, id).unwrap();
    let offset = site.convert(Type::Vec4f, offset).unwrap();
    let mut color = site.add(id, offset).unwrap();
    color = site.add(color, decal).unwrap();
    color = site.add(color, sharp).unwrap();
    let volume = site.convert(Type::Vec3f, volume).unwrap();
    let depth = site.dot(volume, volume).unwrap();
    let levels = site.convert(Type::Float, levels).unwrap();
    let depth = site.add(depth, levels).unwrap();
    color = site.multiply(color, depth).unwrap();
    site.output(color).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "var t_material_ids: texture_2d<u32>;",
        "var t_material_offsets: texture_2d<i32>;",
        "textureSampleLevel(t_material_height, s_pass_linear, input.a_uv, 0.0)",
        "textureDimensions(t_material_height)",
        "textureDimensions(t_material_fog)",
        "textureNumLevels(t_material_height)",
        "textureNumLayers(t_material_decals)",
        "textureLoad(t_material_ids, e6, 0u)",
        "textureLoad(t_material_decals, e6, e9, 0u)",
        "textureSampleBias(t_material_height, s_pass_linear, input.v_uv, -0.5)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn reads_and_queries_take_what_their_texture_does() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let uv = read(&mut site, Frequency::Material, "uv", Type::Vec2f);
    let texel = read(&mut site, Frequency::Material, "texel", Type::Vec2u);
    let level = read(&mut site, Frequency::Material, "level", Type::UInt);
    let sampler = site.resource(Frequency::Pass, "linear", ResourceKind::Sampler).unwrap();
    let flat = site.resource(Frequency::Material, "flat", ResourceKind::Texture2D).unwrap();
    let ids = site.resource(Frequency::Material, "ids", ResourceKind::Texture2Du).unwrap();
    let sky = site.resource(Frequency::Material, "sky", ResourceKind::TextureCube).unwrap();

    let refusals = [
        (
            site.sample(ids, sampler, uv, None),
            "sample(Texture2Du, Sampler, Vec2f) is invalid: expected a float texture: a Texture2Du is read with load",
        ),
        (
            site.sample_level(flat, sampler, uv, None, level),
            "sampleLevel(Texture2D, Sampler, Vec2f, UInt) is invalid: expected sampleLevel(Texture2D, Sampler, \
             Vec2f, a Float level)",
        ),
        (
            site.load_texel(flat, uv, None, level),
            "load(Texture2D, Vec2f, UInt) is invalid: expected load(Texture2D, Vec2i or Vec2u, an Int or UInt level)",
        ),
        (
            site.load_texel(sky, texel, None, level),
            "load(TextureCube, Vec2u, UInt) is invalid: expected a texture that isn't a cube: a cube is only sampled",
        ),
        (
            site.dimensions(sampler, None),
            "dimensions(Sampler) is invalid: expected a texture, and maybe an Int or UInt level",
        ),
        (site.layers(flat), "layers(Texture2D) is invalid: expected an array texture"),
    ];
    for (result, message) in refusals {
        let error = result.unwrap_err();
        assert!(matches!(error.kind, ValidationErrorKind::InvalidTexture { .. }), "{error:?}");
        assert_eq!(error.kind.to_string(), message);
    }
}

#[test]
fn a_bias_needs_the_fragment_stage() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let uv = site.attribute(Rate::Vertex, "uv", Type::Vec2f).unwrap();
    let uv = site.read_attribute(uv);
    let height = site.resource(Frequency::Material, "height", ResourceKind::Texture2D).unwrap();
    let sampler = site.resource(Frequency::Material, "height", ResourceKind::Sampler).unwrap();
    let bias = site.float(1.0).unwrap();
    let lifted = site.sample_bias(height, sampler, uv, None, bias).unwrap();
    site.set_position(lifted).unwrap();
    let white = site.construct(Type::Vec4f, &[bias]).unwrap();
    site.output(white).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "sampleBias is used in the vertex stage, which has no derivatives to pick a mip level with: sample at a \
          level with sampleLevel"
        ]
    );
}

#[test]
fn depth_textures_compare_through_comparison_samplers() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let shadow = site
        .resource(Frequency::Pass, "shadow", ResourceKind::DepthTexture2D)
        .unwrap();
    let cascades = site
        .resource(Frequency::Pass, "cascades", ResourceKind::DepthTexture2DArray)
        .unwrap();
    let point = site
        .resource(Frequency::Pass, "point", ResourceKind::DepthTextureCube)
        .unwrap();
    let compare = site
        .resource(Frequency::Pass, "compare", ResourceKind::ComparisonSampler)
        .unwrap();
    let cascade = read(&mut site, Frequency::Draw, "cascade", Type::UInt);

    // In the vertex stage, a comparison happens in level 0.
    let uv = site.swizzle(position, Swizzle::parse("xy").unwrap()).unwrap();
    let depth = site.swizzle(position, Swizzle::parse("z").unwrap()).unwrap();
    let lit = site.sample_compare_level(shadow, compare, uv, None, depth).unwrap();
    assert_eq!(lit.ty(), Type::Float);
    let lit = site.construct(Type::Vec4f, &[lit]).unwrap();
    let position = site.multiply(position, lit).unwrap();
    site.set_position(position).unwrap();
    let direction = site.swizzle(position, Swizzle::parse("xyz").unwrap()).unwrap();
    let direction = site.pass("direction", direction).unwrap();

    let uv = site.swizzle(direction, Swizzle::parse("xy").unwrap()).unwrap();
    let depth = site.swizzle(direction, Swizzle::parse("z").unwrap()).unwrap();
    let near = site.sample_compare(cascades, compare, uv, Some(cascade), depth).unwrap();
    let far = site.sample_compare(point, compare, direction, None, depth).unwrap();
    let zero = site.uint(0);
    let texel = site.convert(Type::Vec2u, uv).unwrap();
    let stored = site.load_texel(shadow, texel, None, zero).unwrap();
    assert_eq!(stored.ty(), Type::Float);
    let light = site.multiply(near, far).unwrap();
    let light = site.add(light, stored).unwrap();
    let color = site.construct(Type::Vec4f, &[light]).unwrap();
    site.output(color).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "var t_pass_shadow: texture_depth_2d;",
        "var t_pass_cascades: texture_depth_2d_array;",
        "var t_pass_point: texture_depth_cube;",
        "var s_pass_compare: sampler_comparison;",
        "textureSampleCompareLevel(t_pass_shadow, s_pass_compare, ",
        "textureSampleCompare(t_pass_cascades, s_pass_compare, e0, u_draw.f_cascade, e1)",
        "textureSampleCompare(t_pass_point, s_pass_compare, input.v_direction, e1)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn a_comparison_takes_a_depth_texture_and_a_comparison_sampler() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let uv = read(&mut site, Frequency::Material, "uv", Type::Vec2f);
    let depth = read(&mut site, Frequency::Material, "depth", Type::Float);
    let linear = site.resource(Frequency::Pass, "linear", ResourceKind::Sampler).unwrap();
    let compare = site
        .resource(Frequency::Pass, "compare", ResourceKind::ComparisonSampler)
        .unwrap();
    let shadow = site
        .resource(Frequency::Pass, "shadow", ResourceKind::DepthTexture2D)
        .unwrap();
    let flat = site.resource(Frequency::Pass, "flat", ResourceKind::Texture2D).unwrap();

    let refusals = [
        (
            site.sample(shadow, linear, uv, None),
            "sample(DepthTexture2D, Sampler, Vec2f) is invalid: expected a float texture: a DepthTexture2D is \
             compared with sampleCompare, or read with load",
        ),
        (
            site.sample(flat, compare, uv, None),
            "sample(Texture2D, ComparisonSampler, Vec2f) is invalid: expected sample(Texture2D, Sampler, Vec2f)",
        ),
        (
            site.sample_compare(shadow, linear, uv, None, depth),
            "sampleCompare(DepthTexture2D, Sampler, Vec2f, Float) is invalid: expected \
             sampleCompare(DepthTexture2D, ComparisonSampler, Vec2f, a Float depth)",
        ),
        (
            site.sample_compare(flat, compare, uv, None, depth),
            "sampleCompare(Texture2D, ComparisonSampler, Vec2f, Float) is invalid: expected a depth texture: \
             sampleCompare compares depths",
        ),
        (
            site.sample_compare_level(shadow, compare, uv, None, uv),
            "sampleCompareLevel(DepthTexture2D, ComparisonSampler, Vec2f, Vec2f) is invalid: expected \
             sampleCompareLevel(DepthTexture2D, ComparisonSampler, Vec2f, a Float depth)",
        ),
    ];
    for (result, message) in refusals {
        let error = result.unwrap_err();
        assert!(matches!(error.kind, ValidationErrorKind::InvalidTexture { .. }), "{error:?}");
        assert_eq!(error.kind.to_string(), message);
    }
}

#[test]
fn a_comparison_at_the_derivatives_level_needs_the_fragment_stage() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let shadow = site
        .resource(Frequency::Pass, "shadow", ResourceKind::DepthTexture2D)
        .unwrap();
    let compare = site
        .resource(Frequency::Pass, "shadow", ResourceKind::ComparisonSampler)
        .unwrap();
    let uv = site.swizzle(position, Swizzle::parse("xy").unwrap()).unwrap();
    let depth = site.swizzle(position, Swizzle::parse("z").unwrap()).unwrap();
    let lit = site.sample_compare(shadow, compare, uv, None, depth).unwrap();
    let lit = site.construct(Type::Vec4f, &[lit]).unwrap();
    site.set_position(lit).unwrap();
    let one = site.float(1.0).unwrap();
    let white = site.construct(Type::Vec4f, &[one]).unwrap();
    site.output(white).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "sampleCompare is used in the vertex stage, which has no derivatives to pick a mip level with: compare \
          in level 0 with sampleCompareLevel"
        ]
    );
}
