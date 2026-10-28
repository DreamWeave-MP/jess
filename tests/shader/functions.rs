//! The built-in functions: every one called the way it is meant to be, checked by naga, and
//! refused the ways it isn't.

use super::common::{assert_valid_wgsl, read};

use jess::shader::wgsl;
use jess::shader::{Frequency, Function, ProgramBuilder, Rate, Site, Swizzle, Type, ValidationErrorKind, Value};

/// `value` as one `Float`, so every call's result reaches the output.
fn scalar(site: &mut Site<'_>, value: Value) -> Value {
    match value.ty() {
        Type::Float => value,
        Type::Mat3f | Type::Mat4f => site.call(Function::Determinant, &[value]).unwrap(),
        ty if ty.is_vector() => {
            let first = site.swizzle(value, Swizzle::parse("x").unwrap()).unwrap();
            scalar(site, first)
        }
        _ => site.convert(Type::Float, value).unwrap(),
    }
}

#[test]
fn every_function_lowers_to_wgsl_naga_accepts() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    site.set_position(position).unwrap();
    let varying = |site: &mut Site<'_>, name: &str, ty: Type| {
        let attribute = site.attribute(Rate::Vertex, name, ty).unwrap();
        let attribute = site.read_attribute(attribute);
        site.pass(name, attribute).unwrap()
    };
    let shade = varying(&mut site, "shade", Type::Float);
    let normal = varying(&mut site, "normal", Type::Vec3f);
    let color = varying(&mut site, "color", Type::Vec4f);
    let cell = varying(&mut site, "cell", Type::Vec3i);
    let count = read(&mut site, Frequency::Material, "count", Type::UInt);
    let basis = read(&mut site, Frequency::Material, "basis", Type::Mat3f);

    let calls = [
        (Function::Min, vec![normal, shade]),
        (Function::Max, vec![shade, shade]),
        (Function::Clamp, vec![color, shade, shade]),
        (Function::Abs, vec![normal]),
        (Function::Sign, vec![cell]),
        (Function::Floor, vec![normal]),
        (Function::Ceil, vec![shade]),
        (Function::Round, vec![color]),
        (Function::Trunc, vec![normal]),
        (Function::Fract, vec![shade]),
        (Function::Sqrt, vec![shade]),
        (Function::InverseSqrt, vec![normal]),
        (Function::Exp, vec![shade]),
        (Function::Exp2, vec![normal]),
        (Function::Log, vec![shade]),
        (Function::Log2, vec![color]),
        (Function::Pow, vec![normal, shade]),
        (Function::Sin, vec![shade]),
        (Function::Cos, vec![normal]),
        (Function::Tan, vec![shade]),
        (Function::Asin, vec![shade]),
        (Function::Acos, vec![normal]),
        (Function::Atan, vec![shade]),
        (Function::Atan2, vec![shade, shade]),
        (Function::Radians, vec![shade]),
        (Function::Degrees, vec![normal]),
        (Function::Saturate, vec![color]),
        (Function::Step, vec![shade, normal]),
        (Function::Smoothstep, vec![shade, shade, normal]),
        (Function::Mix, vec![normal, normal, shade]),
        (Function::Dot, vec![normal, normal]),
        (Function::Cross, vec![normal, normal]),
        (Function::Length, vec![color]),
        (Function::Distance, vec![normal, normal]),
        (Function::Normalize, vec![normal]),
        (Function::Reflect, vec![normal, normal]),
        (Function::Refract, vec![normal, normal, shade]),
        (Function::Transpose, vec![basis]),
        (Function::Determinant, vec![basis]),
        (Function::Dpdx, vec![shade]),
        (Function::Dpdy, vec![normal]),
        (Function::Fwidth, vec![color]),
    ];
    let called: Vec<Function> = calls.iter().map(|(function, _)| *function).collect();
    assert_eq!(called, Function::ALL, "every function, in order");

    let mut total = site.call(Function::Abs, &[count]).unwrap();
    total = scalar(&mut site, total);
    for (function, arguments) in calls {
        let result = site
            .call(function, &arguments)
            .unwrap_or_else(|error| panic!("{}: {error}", function.name()));
        let result = scalar(&mut site, result);
        total = site.add(total, result).unwrap();
    }
    let color = site.construct(Type::Vec4f, &[total]).unwrap();
    site.output(color).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "min(input.v_normal, vec3<f32>(input.v_shade))",
        "pow(input.v_normal, vec3<f32>(input.v_shade))",
        "step(vec3<f32>(input.v_shade), input.v_normal)",
        "smoothstep(vec3<f32>(input.v_shade), vec3<f32>(input.v_shade), input.v_normal)",
        "inverseSqrt(input.v_normal)",
        "atan2(input.v_shade, input.v_shade)",
        "refract(input.v_normal, input.v_normal, input.v_shade)",
        "transpose(u_material.f_basis)",
        "abs(u_material.f_count)",
        "fwidth(input.v_color)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn functions_refuse_what_they_do_not_take() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let shade = read(&mut site, Frequency::Material, "shade", Type::Float);
    let normal = read(&mut site, Frequency::Material, "normal", Type::Vec3f);
    let uv = read(&mut site, Frequency::Material, "uv", Type::Vec2f);
    let count = read(&mut site, Frequency::Material, "count", Type::UInt);
    let cell = read(&mut site, Frequency::Material, "cell", Type::Vec3i);
    let basis = read(&mut site, Frequency::Material, "basis", Type::Mat3f);

    let refusals = [
        (
            site.call(Function::Sign, &[count]),
            "sign(UInt) is invalid: expected a signed or float scalar or vector",
        ),
        (
            site.call(Function::Abs, &[basis]),
            "abs(Mat3f) is invalid: expected a numeric scalar or vector",
        ),
        (
            site.call(Function::Sqrt, &[cell]),
            "sqrt(Vec3i) is invalid: expected a Float or float vector",
        ),
        (
            site.call(Function::Pow, &[normal, uv]),
            "pow(Vec3f, Vec2f) is invalid: expected two Floats or float vectors of the same type, or a float \
             vector and a Float",
        ),
        (
            site.call(Function::Step, &[cell, normal]),
            "step(Vec3i, Vec3f) is invalid: expected two Floats or float vectors of the same type, or a float \
             vector and a Float",
        ),
        (
            site.call(Function::Smoothstep, &[uv, shade, normal]),
            "smoothstep(Vec2f, Float, Vec3f) is invalid: expected two edges and a Float or float vector, each \
             edge of its type or a Float",
        ),
        (
            site.call(Function::Distance, &[normal, shade]),
            "distance(Vec3f, Float) is invalid: expected two Floats or float vectors of the same type",
        ),
        (
            site.call(Function::Reflect, &[shade, shade]),
            "reflect(Float, Float) is invalid: expected two float vectors of the same type",
        ),
        (
            site.call(Function::Refract, &[normal, normal, normal]),
            "refract(Vec3f, Vec3f, Vec3f) is invalid: expected two float vectors of the same type and a Float",
        ),
        (
            site.call(Function::Determinant, &[normal]),
            "determinant(Vec3f) is invalid: expected a matrix",
        ),
        (
            site.call(Function::Sin, &[shade, shade]),
            "sin(Float, Float) is invalid: expected one argument",
        ),
    ];
    for (result, message) in refusals {
        let error = result.unwrap_err();
        assert!(
            matches!(error.kind, ValidationErrorKind::InvalidOperation { .. }),
            "{error:?}"
        );
        assert_eq!(error.kind.to_string(), message);
    }
}

#[test]
fn derivatives_are_taken_in_the_fragment_stage() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    let moved = site.call(Function::Dpdx, &[position]).unwrap();
    site.set_position(moved).unwrap();
    let one = site.float(1.0).unwrap();
    let white = site.construct(Type::Vec4f, &[one]).unwrap();
    site.output(white).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        ["dpdx is used in the vertex stage: derivatives are taken in the fragment stage"]
    );
}
