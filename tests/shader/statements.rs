//! Stage bodies: vars, ifs, bounded loops, and discard, each value read only where it exists, and
//! derivatives taken only where every fragment agrees on the path.

use super::common::{assert_valid_wgsl, read};

use jess::shader::wgsl;
use jess::shader::{
    BinaryOperator, Frequency, ProgramBuilder, Rate, ResourceKind, Site, Stage, Swizzle, Type, ValidationErrorKind, Value,
};

/// Enters the vertex stage, places `position` at the clip position it declares, and passes `uv`
/// on; returns the fragment stage's `uv`.
fn vertex(site: &mut Site<'_>) -> Value {
    site.enter(Stage::Vertex).unwrap();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    site.set_position(position).unwrap();
    let uv = site.swizzle(position, Swizzle::parse("xy").unwrap()).unwrap();
    let uv = site.pass("uv", uv).unwrap();
    site.enter(Stage::Fragment).unwrap();
    uv
}

#[test]
fn bodies_lower_to_vars_ifs_loops_and_discard() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let count = read(&mut site, Frequency::Material, "count", Type::UInt);
    let diffuse = site
        .resource(Frequency::Material, "diffuse", ResourceKind::Texture2D)
        .unwrap();
    let sampler = site.resource(Frequency::Material, "diffuse", ResourceKind::Sampler).unwrap();
    let uv = vertex(&mut site);

    let zero = site.float(0.0).unwrap();
    let black = site.construct(Type::Vec4f, &[zero]).unwrap();
    let color = site.var(black).unwrap();
    // A loop over a uniform count samples: every fragment runs it the same number of times.
    let index = site.begin_loop(count).unwrap();
    assert_eq!(index.ty(), Type::UInt);
    let step = site.convert(Type::Float, index).unwrap();
    let shifted = site.add(uv, step).unwrap();
    let texel = site.sample(diffuse, sampler, shifted, None).unwrap();
    let sum = site.read_local(color).unwrap();
    let sum = site.add(sum, texel).unwrap();
    site.store(color, sum).unwrap();
    site.end_loop().unwrap();

    let x = site.swizzle(uv, Swizzle::parse("x").unwrap()).unwrap();
    let half = site.float(0.5).unwrap();
    let left = site.binary(BinaryOperator::Less, x, half).unwrap();
    site.begin_if(left).unwrap();
    site.discard().unwrap();
    site.begin_else().unwrap();
    let total = site.read_local(color).unwrap();
    let dimmed = site.multiply(total, half).unwrap();
    site.store(color, dimmed).unwrap();
    site.end_if().unwrap();
    let result = site.read_local(color).unwrap();
    site.output(result).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    let fragment = module.source.split("@fragment").nth(1).unwrap();
    let expected = "
    var l0: vec4<f32>;
    var l1: u32;
    let e0 = vec4<f32>(0.0);
    l0 = e0;
    for (l1 = 0u; l1 < u_material.f_count; l1 = l1 + 1u) {
        let e1 = l1;
        let e2 = f32(e1);
        let e3 = input.v_uv + e2;
        let e4 = textureSample(t_material_diffuse, s_material_diffuse, e3);
        let e5 = l0;
        let e6 = e5 + e4;
        l0 = e6;
    }
    let e7 = input.v_uv.x;
    let e8 = e7 < 0.5;
    if e8 {
        discard;
    } else {
        let e9 = l0;
        let e10 = e9 * 0.5;
        l0 = e10;
    }
    let e11 = l0;
";
    assert!(fragment.contains(expected), "no {expected} in:\n{fragment}");
}

#[test]
fn a_value_is_read_where_it_exists() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let tint = read(&mut site, Frequency::Material, "tint", Type::Vec4f);
    site.enter(Stage::Vertex).unwrap();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let doubled = site.add(tint, tint).unwrap();
    site.set_position(position).unwrap();
    site.enter(Stage::Fragment).unwrap();
    let on = site.bool(true);
    site.begin_if(on).unwrap();
    let inside = site.multiply(tint, tint).unwrap();
    site.end_if().unwrap();
    let both = site.add(inside, doubled).unwrap();
    site.output(both).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "a value computed inside a shader.when or shader.loop is read after it: keep it in a shader.var \
             declared before",
            "a value computed in the vertex function is read in the fragment function: return it from the vertex \
             function, to pass it as a varying",
        ]
    );
}

#[test]
fn statements_take_what_they_need() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let tint = read(&mut site, Frequency::Material, "tint", Type::Vec4f);
    let scale = read(&mut site, Frequency::Material, "scale", Type::Float);
    assert_eq!(
        site.var(tint).unwrap_err().to_string(),
        "the statements don't nest: a statement outside the vertex and fragment functions"
    );
    site.enter(Stage::Vertex).unwrap();
    assert_eq!(
        site.discard().unwrap_err().to_string(),
        "shader.discard is used in the vertex function: only a fragment is discarded"
    );
    let color = site.var(tint).unwrap();
    let refusals = [
        (
            site.store(color, scale).unwrap_err(),
            "a shader.var of Vec4f is set to a Float",
        ),
        (
            site.begin_if(scale).unwrap_err(),
            "shader.when's condition is a Float: expected a Bool",
        ),
        (
            site.begin_loop(scale).unwrap_err(),
            "shader.loop's count is a Float: expected an Int or UInt",
        ),
        (
            site.end_loop().unwrap_err(),
            "the statements don't nest: an end of a shader.loop that isn't open",
        ),
    ];
    for (error, message) in refusals {
        assert_eq!(error.to_string(), message);
    }
    site.enter(Stage::Fragment).unwrap();
    assert!(matches!(
        site.read_local(color).unwrap_err().kind,
        ValidationErrorKind::LocalInOtherStage { .. }
    ));
    assert_eq!(
        site.read_local(color).unwrap_err().to_string(),
        "a shader.var of the vertex function is used in the fragment function"
    );
}

#[test]
fn derivatives_need_every_fragment_on_the_path() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let diffuse = site
        .resource(Frequency::Material, "diffuse", ResourceKind::Texture2D)
        .unwrap();
    let sampler = site.resource(Frequency::Material, "diffuse", ResourceKind::Sampler).unwrap();
    let uv = vertex(&mut site);
    let x = site.swizzle(uv, Swizzle::parse("x").unwrap()).unwrap();
    let half = site.float(0.5).unwrap();

    // A var some fragments set differently makes whatever it decides vary too.
    let zero = site.float(0.0).unwrap();
    let edge = site.var(zero).unwrap();
    site.store(edge, x).unwrap();
    let edge = site.read_local(edge).unwrap();
    let right = site.binary(BinaryOperator::Greater, edge, half).unwrap();
    let one = site.float(1.0).unwrap();
    let white = site.construct(Type::Vec4f, &[one]).unwrap();
    let mixed = site.var(white).unwrap();
    site.begin_if(right).unwrap();
    let texel = site.sample(diffuse, sampler, uv, None).unwrap();
    // A level the shader names needs no derivatives, so it may go anywhere.
    let level = site.sample_level(diffuse, sampler, uv, None, zero).unwrap();
    let both = site.add(texel, level).unwrap();
    let slope = site.call(jess::shader::Function::Dpdx, &[x]).unwrap();
    let brighter = site.multiply(both, slope).unwrap();
    site.store(mixed, brighter).unwrap();
    site.end_if().unwrap();
    let mixed = site.read_local(mixed).unwrap();
    site.output(mixed).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "sample is used where fragments may take different paths, in a shader.when or shader.loop decided by \
             a value that varies per fragment, and derivatives need them all: sample before the shader.when or \
             shader.loop, or sample at a level with sampleLevel",
            "dpdx is used where fragments may take different paths, in a shader.when or shader.loop decided by a \
             value that varies per fragment, and derivatives need them all: take it before the shader.when or \
             shader.loop",
        ]
    );
}

#[test]
fn folding_keeps_what_statements_read() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let tint = read(&mut site, Frequency::Material, "tint", Type::Vec4f);
    vertex(&mut site);
    let one = site.float(1.0).unwrap();
    let same = site.multiply(tint, one).unwrap();
    let color = site.var(same).unwrap();
    let color = site.read_local(color).unwrap();
    site.output(color).unwrap();

    let mut program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(program.fold_constants(), Ok(1));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    assert!(module.source.contains("    l0 = u_material.f_tint;\n"), "{}", module.source);
}
