//! Built-in values: what the pipeline gives each stage, read only where it exists, and lowered to
//! entry point parameters only when a stage reads them.

use super::common::assert_valid_wgsl;

use jess::shader::wgsl;
use jess::shader::{Builtin, ProgramBuilder, Rate, Swizzle, Type};

#[test]
fn each_stage_reads_the_values_the_pipeline_gives_it() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec3f).unwrap();
    let position = site.read_attribute(position);
    let vertex = site.builtin(Builtin::VertexIndex);
    let instance = site.builtin(Builtin::InstanceIndex);
    assert_eq!(vertex.ty(), Type::UInt);
    let offset = site.add(vertex, instance).unwrap();
    let offset = site.convert(Type::Float, offset).unwrap();
    let one = site.float(1.0).unwrap();
    let clip = site.construct(Type::Vec4f, &[position, one]).unwrap();
    let shifted = site.add(clip, offset).unwrap();
    site.set_position(shifted).unwrap();

    let pixel = site.builtin(Builtin::Position);
    assert_eq!(pixel.ty(), Type::Vec4f);
    let facing = site.builtin(Builtin::FrontFacing);
    assert_eq!(facing.ty(), Type::Bool);
    let depth = site.swizzle(pixel, Swizzle::parse("zzzw").unwrap()).unwrap();
    let back = site.construct(Type::Vec4f, &[one]).unwrap();
    let color = site.select(facing, depth, back).unwrap();
    site.output(color).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "fn vertex_main(input: VertexInput, @builtin(vertex_index) vertex_index: u32, @builtin(instance_index) \
         instance_index: u32) -> VertexOutput {",
        "let e0 = vertex_index + instance_index;",
        "fn fragment_main(input: VertexOutput, @builtin(front_facing) front_facing: bool) -> FragmentOutput {",
        "input.position.zzzw",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn a_program_that_reads_none_takes_no_parameters_for_them() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let position = site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let position = site.read_attribute(position);
    site.set_position(position).unwrap();
    let one = site.float(1.0).unwrap();
    let white = site.construct(Type::Vec4f, &[one]).unwrap();
    site.output(white).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert!(module.source.contains("fn vertex_main(input: VertexInput) -> VertexOutput {"));
    assert!(
        module
            .source
            .contains("fn fragment_main(input: VertexOutput) -> FragmentOutput {")
    );
}

#[test]
fn a_builtin_is_read_only_in_the_stage_that_has_it() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let pixel = site.builtin(Builtin::Position);
    site.set_position(pixel).unwrap();
    let index = site.builtin(Builtin::InstanceIndex);
    let shade = site.convert(Type::Float, index).unwrap();
    let color = site.construct(Type::Vec4f, &[shade]).unwrap();
    site.output(color).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "builtin position is read in the vertex stage: only the fragment stage has it",
            "builtin instanceIndex is read in the fragment stage: only the vertex stage has it",
        ]
    );
}
