//! The Luau shader corpus (`tests/shader/*.jsl`), each an ordinary module returning a
//! `ShaderProgram`, loaded under that contract and taken all the way through: `@jess/shader`,
//! `jess::shader::Program`, constant folding, WGSL, and naga's validation. The invalid modules fail
//! where they're written, in Jess terms.

use std::path::{Path, PathBuf};

use jess::lua::l3i::Runtime;
use jess::lua::scripts::load_module;
use jess::lua::shader::LuaShaderProgram;
use jess::shader::wgsl::{self, WgslModule};
use jess::shader::{Program, Type};

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

#[test]
fn solid_color_lowers_to_valid_wgsl() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "solid-color.jsl");
    assert_eq!(declared(program.parameters()), [("tint", Type::Vec4f)]);
    assert_eq!(declared(program.vertex_inputs()), [("position", Type::Vec3f)]);
    assert_eq!(program.varyings(), []);
    assert_eq!(module.fragment_outputs, 1);
    assert!(module.source.contains("parameters.p_tint"), "{}", module.source);
}

#[test]
fn vertex_transform_passes_a_varying_and_keeps_luau_evaluation_order() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "vertex-transform.jsl");
    assert_eq!(
        declared(program.vertex_inputs()),
        [("position", Type::Vec3f), ("color", Type::Vec4f)]
    );
    assert_eq!(declared(program.varyings()), [("color", Type::Vec4f)]);
    // Luau multiplies left to right: the matrix products come first.
    assert!(
        module.source.contains("semantics.projection * semantics.view"),
        "{}",
        module.source
    );
    assert!(module.source.contains("output.v_color = input.a_color;"), "{}", module.source);
    assert!(
        module.source.contains("input.v_color * parameters.p_tint"),
        "{}",
        module.source
    );
}

#[test]
fn textured_samples_with_the_sampler_declared_with_the_texture() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "textured.jsl");
    assert_eq!(
        declared(program.parameters()),
        [("diffuse", Type::Texture2D), ("diffuse", Type::Sampler)]
    );
    assert_eq!(declared(program.varyings()), [("uv", Type::Vec2f)]);
    assert_eq!(module.resources.len(), 2);
    assert!(
        module.source.contains("textureSample(t_diffuse, s_diffuse, input.v_uv)"),
        "{}",
        module.source
    );
}

#[test]
fn bayer_dither_port_builds_every_static_mode_and_lowers_to_valid_wgsl() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "bayer-dither.jsl");
    assert_eq!(
        declared(program.parameters()),
        [("source", Type::Texture2D), ("source", Type::Sampler)]
    );
    assert_eq!(declared(program.varyings()), [("uv", Type::Vec2f)]);
    assert!(
        module.source.contains("textureSample(t_source, s_source, input.v_uv)"),
        "{}",
        module.source
    );
    assert!(module.source.contains("semantics.viewport_size"), "{}", module.source);
    assert!(module.source.contains("floor("), "{}", module.source);
}

#[test]
fn directional_lighting_lowers_to_valid_wgsl() {
    let runtime = common::runtime();
    let (program, module) = lowered(&runtime, "directional-lit.jsl");
    // Parameters are declared sorted by name, then the textures with their samplers.
    assert_eq!(
        declared(program.parameters()),
        [
            ("ambient", Type::Float),
            ("lightDirection", Type::Vec3f),
            ("normalMatrix", Type::Mat3f),
            ("diffuse", Type::Texture2D),
            ("diffuse", Type::Sampler),
        ]
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
        format!("{file}:15 ({file}): dot(Vec3f, Vec4f) is invalid: expected two float vectors of the same size")
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
