//! Compute: a compute program run over workgroups, reading and writing storage buffers, read back.

use super::gpu;

use jess::gpu::{Binding, BufferUsage, GpuError};
use jess::shader::{
    Access, BinaryOperator, Builtin, DataType, Element, Frequency, Place, PlaceRoot, Program, ProgramBuilder, Stage, Step,
    StorageId, Swizzle, Type,
};

/// Moves each position by its velocity times the step, for as many as the buffer holds; with the
/// positions' and velocities' storage buffers.
fn integrate() -> (Program, [StorageId; 2]) {
    let mut builder = ProgramBuilder::compute([64, 1, 1]);
    let mut site = builder.site();
    let positions = site
        .storage(Frequency::Pass, "positions", Element::Value(Type::Vec4f), Access::ReadWrite)
        .unwrap();
    let velocities = site
        .storage(Frequency::Pass, "velocities", Element::Value(Type::Vec4f), Access::Read)
        .unwrap();
    site.uniform(Frequency::Pass, "step", DataType::Value(Type::Float)).unwrap();
    site.enter(Stage::Compute).unwrap();
    let invocation = site.builtin(Builtin::GlobalInvocationId);
    let index = site.swizzle(invocation, Swizzle::parse("x").unwrap()).unwrap();
    let count = site.storage_length(positions).unwrap();
    let inside = site.binary(BinaryOperator::Less, index, count).unwrap();
    site.begin_if(inside).unwrap();
    let at = |storage| Place::new(PlaceRoot::Storage(storage), &[Step::Index(index.expression())]).unwrap();
    let position = site.load(at(positions), &[index]).unwrap();
    let velocity = site.load(at(velocities), &[index]).unwrap();
    let step = Place::new(PlaceRoot::Uniform(Frequency::Pass), &[Step::Field(0)]).unwrap();
    let step = site.load(step, &[]).unwrap();
    let moved = site.multiply(velocity, step).unwrap();
    let moved = site.add(position, moved).unwrap();
    site.write(at(positions), moved, &[index]).unwrap();
    site.end_if().unwrap();
    (builder.finish().unwrap(), [positions, velocities])
}

fn floats(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

#[test]
fn a_dispatch_writes_storage_buffers() {
    let Some(gpu) = gpu("a_dispatch_writes_storage_buffers") else {
        return;
    };
    let (program, [positions_id, velocities_id]) = integrate();
    let pipeline = gpu.compute_pipeline(&program).unwrap();
    assert_eq!(pipeline.workgroup_size(), [64, 1, 1]);
    // 100 particles: two workgroups, the second only partly inside the buffer.
    let count = 100usize;
    let positions: Vec<f32> = (0..400u16).map(f32::from).collect();
    let velocities = vec![1.0f32; count * 4];
    let bytes = |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|value| value.to_le_bytes()).collect() };
    let positions_buffer = gpu.buffer((count * 16) as u64, BufferUsage::STORAGE).unwrap();
    gpu.write_buffer(&positions_buffer, 0, &bytes(&positions)).unwrap();
    let velocities_buffer = gpu.buffer((count * 16) as u64, BufferUsage::STORAGE).unwrap();
    gpu.write_buffer(&velocities_buffer, 0, &bytes(&velocities)).unwrap();
    let step = gpu.buffer(16, BufferUsage::UNIFORM).unwrap();
    gpu.write_buffer(&step, 0, &bytes(&[0.5, 0.0, 0.0, 0.0])).unwrap();
    let pass = gpu
        .compute_bind_group(
            &pipeline,
            Frequency::Pass,
            &[
                Binding::Uniform(&step),
                Binding::Storage(positions_id, &positions_buffer),
                Binding::Storage(velocities_id, &velocities_buffer),
            ],
        )
        .unwrap();
    gpu.dispatch(&pipeline, [Some(&pass), None, None], [2, 1, 1]).unwrap();
    let moved = floats(&gpu.read_buffer(&positions_buffer, 0, (count * 16) as u64).unwrap());
    let expected: Vec<f32> = positions.iter().map(|position| position + 0.5).collect();
    assert_eq!(moved, expected);

    let refusals = [
        (
            gpu.dispatch(&pipeline, [None; 3], [1, 1, 1]).unwrap_err(),
            "the dispatch is invalid: its program binds a pass group, and the dispatch gives none",
        ),
        (
            gpu.dispatch(&pipeline, [Some(&pass), None, None], [0, 1, 1]).unwrap_err(),
            "a dispatch of 0×1×1 workgroups is invalid: expected from 1 to",
        ),
        (
            gpu.compute_pipeline(&Program::new()).unwrap_err(),
            "a vertex and fragment program draws: make it a render pipeline",
        ),
    ];
    for (error, message) in refusals {
        assert!(matches!(error, GpuError::Invalid(_)), "{error:?}");
        assert!(error.to_string().starts_with(message), "{error} doesn't start with {message}");
    }
}
