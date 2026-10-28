//! The Luau shader corpus (`tests/shader/*.jsl`), each an ordinary module returning a
//! `ShaderProgram`, loaded under that contract and taken all the way through: `@jess/shader`,
//! `jess::shader::Program`, constant folding, WGSL, and naga's validation. The invalid modules fail
//! where they're written, in Jess terms.

use std::path::{Path, PathBuf};

use jess::lua::l3i::Runtime;
use jess::lua::scripts::load_module;
use jess::lua::shader::LuaShaderProgram;
use jess::shader::wgsl::{self, WgslModule};
use jess::shader::{Access, DataType, Frequency, Program, ResourceKind, Type};

use super::common;

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lua/shader").join(name)
}

/// Parses and validates `module`'s WGSL with naga.
fn assert_valid_wgsl(module: &WgslModule) {
    let parsed = match naga::front::wgsl::parse_str(&module.source) {
        Ok(parsed) => parsed,
        Err(error) => panic!(
            "naga cannot parse the WGSL:\n{}\n{}",
            error.emit_to_string(&module.source),
            module.source
        ),
    };
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default());
    if let Err(error) = validator.validate(&parsed) {
        panic!("naga rejects the WGSL: {error:?}\n{}", module.source);
    }
}

/// Loads a corpus module as a shader program, folds it, and lowers it to WGSL naga accepts.
fn lowered(runtime: &Runtime, name: &str) -> (Program, WgslModule) {
    let path = corpus(name);
    let program = load_module::<LuaShaderProgram>(runtime, &path).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(program.owner(), path.display().to_string());
    let mut program = program.program().clone();
    program.fold_constants().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    (program, module)
}

fn declared(declarations: &[jess::shader::Declaration]) -> Vec<(&str, Type)> {
    declarations
        .iter()
        .map(|declaration| (declaration.name.as_str(), declaration.ty))
        .collect()
}

fn attributes(program: &Program) -> Vec<(&str, Type)> {
    program
        .attributes()
        .iter()
        .map(|attribute| (attribute.name.as_str(), attribute.ty))
        .collect()
}

/// The uniform members `frequency` binds, as declared.
fn uniforms(program: &Program, frequency: Frequency) -> Vec<(&str, DataType)> {
    program
        .uniforms_of(frequency)
        .map(|(_, uniform)| (uniform.name.as_str(), uniform.ty))
        .collect()
}

/// The textures and samplers `frequency` binds.
fn resources(program: &Program, frequency: Frequency) -> Vec<(&str, ResourceKind)> {
    program
        .resources()
        .iter()
        .filter(|resource| resource.frequency == frequency)
        .map(|resource| (resource.name.as_str(), resource.kind))
        .collect()
}

#[test]
fn solid_color_lowers_to_valid_wgsl() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "solid-color.jsl");
    assert_eq!(
        uniforms(&program, Frequency::Material),
        [("tint", DataType::Value(Type::Vec4f))]
    );
    assert_eq!(attributes(&program), [("position", Type::Vec3f)]);
    assert_eq!(program.varyings(), []);
    assert_eq!(module.outputs.len(), 1);
    assert!(module.source.contains("u_material.f_tint"), "{}", module.source);
}

#[test]
fn vertex_transform_passes_a_varying_and_keeps_luau_evaluation_order() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "vertex-transform.jsl");
    assert_eq!(attributes(&program), [("color", Type::Vec4f), ("position", Type::Vec3f)]);
    assert_eq!(declared(program.varyings()), [("color", Type::Vec4f)]);
    // Luau multiplies left to right: the matrix products come first.
    assert!(
        module.source.contains("u_pass.f_projection * u_pass.f_view"),
        "{}",
        module.source
    );
    assert!(module.source.contains("output.v_color = input.a_color;"), "{}", module.source);
    assert!(
        module.source.contains("input.v_color * u_material.f_tint"),
        "{}",
        module.source
    );
}

#[test]
fn point_lights_loop_and_discard_in_the_fragment_body() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "point-lights.jsl");
    assert_eq!(program.locals().len(), 2, "the light var and the loop's index");
    for line in [
        "    if e2 {\n        discard;\n    }\n",
        "    for (l1 = 0u; l1 < u_pass.f_lightCount; l1 = l1 + 1u) {\n",
        "        let e5 = u_pass.f_lights[e4].f_position;\n",
        "    l0 = vec3<f32>(0.0, 0.0, 0.0);\n",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn particles_read_one_storage_buffer_and_write_another() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "particles.jsl");
    let buffers: Vec<(&str, Access)> = program
        .storage_buffers()
        .iter()
        .map(|buffer| (buffer.name.as_str(), buffer.access))
        .collect();
    assert_eq!(buffers, [("coverage", Access::ReadWrite), ("particles", Access::Read)]);
    for line in [
        "var<storage, read_write> b_pass_coverage: array<u32>;",
        "var<storage, read> b_material_particles: array<S0_Particle>;",
        "b_material_particles[instance_index].f_size",
        "arrayLength(&b_pass_coverage)",
        "        b_pass_coverage[e5] = e9;\n",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn integrate_is_a_compute_program() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "integrate.jsl");
    assert_eq!(program.workgroup_size(), Some([64, 1, 1]));
    for line in [
        "@compute @workgroup_size(64, 1, 1)\nfn compute_main(@builtin(global_invocation_id) global_invocation_id: vec3<u32>) {",
        "arrayLength(&b_pass_positions)",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn textured_samples_with_the_sampler_declared_with_the_texture() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "textured.jsl");
    assert_eq!(
        resources(&program, Frequency::Material),
        [("diffuse", ResourceKind::Texture2D), ("diffuse", ResourceKind::Sampler)]
    );
    assert_eq!(declared(program.varyings()), [("uv", Type::Vec2f)]);
    assert_eq!(module.layout.group(Frequency::Material).resources.len(), 2);
    assert!(
        module
            .source
            .contains("textureSample(t_material_diffuse, s_material_diffuse, input.v_uv)"),
        "{}",
        module.source
    );
}

#[test]
fn bayer_dither_port_builds_every_static_mode_and_lowers_to_valid_wgsl() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "bayer-dither.jsl");
    assert_eq!(
        resources(&program, Frequency::Material),
        [("source", ResourceKind::Texture2D), ("source", ResourceKind::Sampler)]
    );
    assert_eq!(declared(program.varyings()), [("uv", Type::Vec2f)]);
    assert!(
        module
            .source
            .contains("textureSample(t_material_source, s_material_source, input.v_uv)"),
        "{}",
        module.source
    );
    assert!(module.source.contains("u_pass.f_viewportSize"), "{}", module.source);
    assert!(module.source.contains("floor("), "{}", module.source);
}

#[test]
fn directional_lighting_lowers_to_valid_wgsl() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "directional-lit.jsl");
    // What the material binds is declared sorted by name: uniform members, then the texture with
    // its sampler.
    assert_eq!(
        uniforms(&program, Frequency::Material),
        [
            ("ambient", DataType::Value(Type::Float)),
            ("lightDirection", DataType::Value(Type::Vec3f)),
            ("normalMatrix", DataType::Value(Type::Mat3f)),
        ]
    );
    assert_eq!(
        resources(&program, Frequency::Material),
        [("diffuse", ResourceKind::Texture2D), ("diffuse", ResourceKind::Sampler)]
    );
    assert_eq!(program.structs()[0].name, "Camera");
    assert!(
        module
            .source
            .contains("u_pass.f_camera.f_projection * u_pass.f_camera.f_view")
    );
    assert_eq!(declared(program.varyings()), [("normal", Type::Vec3f), ("uv", Type::Vec2f)]);
    for expected in ["normalize(", "dot(", "max(", "clamp(", ".xyz", ".w"] {
        assert!(module.source.contains(expected), "no {expected} in\n{}", module.source);
    }
}

#[test]
fn an_invalid_dot_fails_at_the_line_that_wrote_it() {
    let runtime = common::runtime();
    let path = corpus("invalid-dot.jsl");
    let error = load_module::<LuaShaderProgram>(&runtime, &path).unwrap_err().to_string();
    let file = path.display().to_string();
    assert_eq!(
        error,
        format!("{file}:22 ({file}): dot(Vec3f, Vec4f) is invalid: expected two vectors of the same type")
    );
}

#[test]
fn a_module_returning_something_else_fails_the_contract() {
    let runtime = common::runtime();
    let path = corpus("wrong-return-type.jsl");
    let error = load_module::<LuaShaderProgram>(&runtime, &path).unwrap_err().to_string();
    assert_eq!(
        error,
        format!(
            "{}: expected the module to return a jess.shader.Program, but it returned a number",
            path.display()
        )
    );
}

#[test]
fn the_contract_names_whatever_came_back() {
    let runtime = common::runtime();
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        ("return nil", "nil"),
        ("", "nil"),
        ("return 'text'", "a string"),
        ("return {}", "a table"),
        ("return true", "a boolean"),
        ("return print", "a function"),
        ("return jess.shader.vec4", "a jess.shader.Type"),
    ];
    for (index, (source, came_back)) in cases.into_iter().enumerate() {
        // Chunks are cached by name per runtime, so each module gets its own.
        let path = directory.path().join(format!("module{index}.jsl"));
        std::fs::write(&path, source).unwrap();
        let error = load_module::<LuaShaderProgram>(&runtime, &path).unwrap_err().to_string();
        assert_eq!(
            error,
            format!(
                "{}: expected the module to return a jess.shader.Program, but it returned {came_back}",
                path.display()
            ),
            "{source}"
        );
    }
    let missing = directory.path().join("missing.jsl");
    let error = load_module::<LuaShaderProgram>(&runtime, &missing).unwrap_err().to_string();
    assert_eq!(error, format!("module not found: {}", missing.display()));
}

/// The corpus type-checks under `--!strict` against the types folder, `require("@jess/shader")`
/// included, and a mistake the types can see is one the checker reports.
#[test]
fn the_corpus_type_checks_against_the_definitions() {
    for entry in std::fs::read_dir(corpus("")).unwrap() {
        let path = entry.unwrap().path();
        let source = std::fs::read_to_string(&path).unwrap();
        let report = common::type_check(&source);
        assert!(report.trim().is_empty(), "{}: {report}", path.display());
    }
    let mistyped = "local shader = require '@jess/shader'\nlocal wrong: number = shader.dot(1, 2)\nshader.nothing()\n";
    let report = common::type_check(mistyped);
    assert!(report.contains("ShaderExpr"), "{report}");
    assert!(report.contains("nothing"), "{report}");
}

#[test]
fn the_module_is_the_same_table_by_either_name() {
    let runtime = common::runtime();
    common::run(&runtime, "assert(require('@jess/shader') == jess.shader)");
}
