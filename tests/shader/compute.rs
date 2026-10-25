//! Compute programs: one stage run per invocation, in workgroups, reading and writing storage
//! buffers, and refusing what only the vertex and fragment stages have.

use super::common::{assert_valid_wgsl, read};

use jess::shader::wgsl;
use jess::shader::{
    Access, BinaryOperator, Builtin, Element, Frequency, Place, PlaceRoot, Program, ProgramBuilder, Rate, ResourceKind,
    Stage, Step, Swizzle, Type, Value,
};

fn element(storage: jess::shader::StorageId, index: Value) -> Place {
    Place::new(PlaceRoot::Storage(storage), &[Step::Index(index.expression())]).unwrap()
}

#[test]
fn a_compute_program_integrates_particles() {
    let mut builder = ProgramBuilder::compute([64, 1, 1]);
    let mut site = builder.site();
    let positions = site
        .storage(Frequency::Pass, "positions", Element::Value(Type::Vec4f), Access::ReadWrite)
        .unwrap();
    let velocities = site
        .storage(Frequency::Pass, "velocities", Element::Value(Type::Vec4f), Access::Read)
        .unwrap();
    let step = read(&mut site, Frequency::Pass, "step", Type::Float);

    site.enter(Stage::Compute).unwrap();
    let invocation = site.builtin(Builtin::GlobalInvocationId);
    assert_eq!(invocation.ty(), Type::Vec3u);
    let index = site.swizzle(invocation, Swizzle::parse("x").unwrap()).unwrap();
    let count = site.storage_length(positions).unwrap();
    let inside = site.binary(BinaryOperator::Less, index, count).unwrap();
    site.begin_if(inside).unwrap();
    let position = site.load(element(positions, index), &[index]).unwrap();
    let velocity = site.load(element(velocities, index), &[index]).unwrap();
    let moved = site.multiply(velocity, step).unwrap();
    let moved = site.add(position, moved).unwrap();
    site.write(element(positions, index), moved, &[index]).unwrap();
    site.end_if().unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_eq!(program.workgroup_size(), Some([64, 1, 1]));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    assert!(!module.source.contains("@vertex"), "{}", module.source);
    for line in [
        "@compute @workgroup_size(64, 1, 1)\nfn compute_main(@builtin(global_invocation_id) global_invocation_id: \
         vec3<u32>) {",
        "    let e0 = global_invocation_id.x;\n",
        "        b_pass_positions[e0] = e6;\n",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
}

#[test]
fn a_compute_program_has_one_stage() {
    let mut builder = ProgramBuilder::compute([16, 16, 2]);
    let mut site = builder.site();
    site.attribute(Rate::Vertex, "position", Type::Vec4f).unwrap();
    let texture = site.resource(Frequency::Pass, "noise", ResourceKind::Texture2D).unwrap();
    let sampler = site.resource(Frequency::Pass, "noise", ResourceKind::Sampler).unwrap();
    site.enter(Stage::Compute).unwrap();
    assert_eq!(
        site.discard().unwrap_err().to_string(),
        "shader.discard is used in the compute function: only a fragment is discarded"
    );
    let uv = site.literal(jess::shader::Literal::Vec2f([0.5, 0.5])).unwrap();
    let texel = site.sample(texture, sampler, uv, None).unwrap();
    site.var(texel).unwrap();
    let facing = site.builtin(Builtin::FrontFacing);
    site.var(facing).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "a workgroup of 16×16×2 is invalid: expected each size at least 1, x and y at most 256, z at most 64, \
             and at most 256 invocations in all",
            "a compute program has no attributes: it runs one function, compute, per invocation",
            "sample is used in the compute stage, which has no derivatives to pick a mip level with: sample at a \
             level with sampleLevel",
            "builtin frontFacing is read in the compute stage: only the fragment stage has it",
        ]
    );
}

#[test]
fn a_render_program_reads_no_invocation() {
    let mut program = Program::new();
    let invocation = program.push(jess::shader::Expression::Builtin(Builtin::LocalInvocationIndex));
    let shade = program.push(jess::shader::Expression::Convert {
        target: Type::Float,
        value: invocation,
    });
    let color = program.push(jess::shader::Expression::construct(Type::Vec4f, &[shade]).unwrap());
    program.set_position(color);
    program
        .body_mut(Stage::Compute)
        .push(jess::shader::Statement::Discard { origin: None });
    let messages: Vec<String> = program.validate().unwrap_err().iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "a vertex and fragment program has compute statements: they belong to a compute program",
            "builtin localInvocationIndex is read in the vertex stage: only the compute stage has it",
            "shader.discard is used in the compute function: only a fragment is discarded",
        ]
    );
}
