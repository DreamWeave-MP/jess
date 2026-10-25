//! Storage buffers: as many elements as the bound buffer holds, read in any stage, written where
//! WGSL allows, laid out where jess says, and checked by naga.

use super::common::{assert_valid_wgsl, read};

use jess::shader::wgsl;
use jess::shader::{
    Access, AtomicOp, Builtin, DataType, Element, Field, Frequency, Place, PlaceRoot, ProgramBuilder, Stage, Step, Type,
    Value,
};

/// `buffer[index]`, then `steps`.
fn element(storage: jess::shader::StorageId, index: Value, steps: &[Step]) -> Place {
    let mut path = vec![Step::Index(index.expression())];
    path.extend_from_slice(steps);
    Place::new(PlaceRoot::Storage(storage), &path).unwrap()
}

#[test]
fn storage_is_read_in_any_stage_and_written_in_the_fragment_stage() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let value = DataType::Value;
    let particle = site
        .structure(
            "Particle",
            vec![
                Field::new("position", value(Type::Vec4f)),
                Field::new("speed", value(Type::Float)),
            ],
        )
        .unwrap();
    let particles = site
        .storage(Frequency::Material, "particles", Element::Struct(particle), Access::Read)
        .unwrap();
    let hits = site
        .storage(Frequency::Pass, "hits", Element::Value(Type::UInt), Access::ReadWrite)
        .unwrap();
    let scale = read(&mut site, Frequency::Material, "scale", Type::Float);

    site.enter(Stage::Vertex).unwrap();
    let instance = site.builtin(Builtin::InstanceIndex);
    let position = site
        .load(element(particles, instance, &[Step::Field(0)]), &[instance])
        .unwrap();
    assert_eq!(position.ty(), Type::Vec4f);
    let speed = site
        .load(element(particles, instance, &[Step::Field(1)]), &[instance])
        .unwrap();
    let speed = site.multiply(speed, scale).unwrap();
    let count = site.storage_length(particles).unwrap();
    assert_eq!(count.ty(), Type::UInt);
    let count = site.convert(Type::Float, count).unwrap();
    let speed = site.add(speed, count).unwrap();
    site.set_position(position).unwrap();
    let speed = site.pass("speed", speed).unwrap();

    site.enter(Stage::Fragment).unwrap();
    let slot = site.convert(Type::UInt, speed).unwrap();
    let seen = site.load(element(hits, slot, &[]), &[slot]).unwrap();
    let one = site.uint(1);
    let more = site.add(seen, one).unwrap();
    site.write(element(hits, slot, &[]), more, &[slot]).unwrap();
    let color = site.construct(Type::Vec4f, &[speed]).unwrap();
    site.output(color).unwrap();

    let program = builder.finish().unwrap_or_else(|errors| panic!("{errors:#?}"));
    let module = wgsl::lower(&program).unwrap_or_else(|errors| panic!("{errors:#?}"));
    assert_valid_wgsl(&module);
    for line in [
        "@group(1) @binding(1) var<storage, read> b_material_particles: array<S0_Particle>;",
        "@group(0) @binding(0) var<storage, read_write> b_pass_hits: array<u32>;",
        "let e0 = b_material_particles[instance_index].f_position;",
        "arrayLength(&b_material_particles)",
        "    b_pass_hits[e0] = e2;\n",
    ] {
        assert!(module.source.contains(line), "no {line:?} in:\n{}", module.source);
    }
    let layout = &module.layout;
    let particles = layout.storage(particles).unwrap();
    assert_eq!(
        (particles.group, particles.binding, particles.stride),
        (1, 1, 32),
        "after the material block; a Vec4f and a Float, padded to the struct's 16-byte alignment"
    );
    assert_eq!(layout.storage(hits).unwrap().stride, 4);
}

#[test]
fn storage_is_written_only_where_it_may_be() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let read_only = site
        .storage(Frequency::Material, "weights", Element::Value(Type::Float), Access::Read)
        .unwrap();
    let written = site
        .storage(Frequency::Material, "sums", Element::Value(Type::Float), Access::ReadWrite)
        .unwrap();
    assert_eq!(
        site.storage(Frequency::Material, "sums", Element::Value(Type::Bool), Access::Read)
            .unwrap_err()
            .to_string(),
        "material storage buffer \"sums\" is declared twice"
    );
    site.uniform(Frequency::Material, "tint", DataType::Value(Type::Vec4f))
        .unwrap();

    site.enter(Stage::Vertex).unwrap();
    let zero = site.uint(0);
    let half = site.float(0.5).unwrap();
    let refusals = [
        (
            site.write(element(written, zero, &[]), half, &[zero]).unwrap_err(),
            "storage buffer \"sums\" is readWrite and used in the vertex function, which may only read storage \
             buffers declared to be read",
        ),
        (
            site.write(element(read_only, zero, &[]), half, &[zero]).unwrap_err(),
            "storage buffer \"weights\" is written, but it is read: declare it shader.storage(element, 'readWrite')",
        ),
        (
            site.write(
                Place::new(PlaceRoot::Uniform(Frequency::Material), &[Step::Field(0)]).unwrap(),
                half,
                &[],
            )
            .unwrap_err(),
            "a uniform is written: only a readWrite storage buffer is",
        ),
    ];
    for (error, message) in refusals {
        assert_eq!(error.to_string(), message);
    }
    let sum = site.load(element(written, zero, &[]), &[zero]).unwrap();
    let position = site.construct(Type::Vec4f, &[sum]).unwrap();
    site.set_position(position).unwrap();

    site.enter(Stage::Fragment).unwrap();
    let one = site.uint(1);
    assert_eq!(
        site.write(element(written, one, &[]), one, &[one]).unwrap_err().to_string(),
        "a Float in a storage buffer is written a UInt"
    );
    let white = site.construct(Type::Vec4f, &[half]).unwrap();
    site.output(white).unwrap();

    let errors = builder.finish().unwrap_err();
    let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        messages,
        [
            "storage buffer \"sums\" is readWrite and used in the vertex function, which may only read storage buffers \
          declared to be read"
        ]
    );
}

/// An atomic storage buffer is read, written, and updated only through WGSL's atomics, every
/// update giving back what the element held; nothing else is updated atomically.
#[test]
fn atomic_storage_is_reached_only_through_atomics() {
    let mut builder = ProgramBuilder::compute([64, 1, 1]);
    let mut site = builder.site();
    let counts = site
        .storage(Frequency::Pass, "counts", Element::Value(Type::UInt), Access::Atomic)
        .unwrap();
    let kept = site
        .storage(Frequency::Pass, "kept", Element::Value(Type::UInt), Access::ReadWrite)
        .unwrap();
    assert_eq!(
        site.storage(Frequency::Pass, "levels", Element::Value(Type::Float), Access::Atomic)
            .unwrap_err()
            .to_string(),
        "atomic storage buffer \"levels\" holds something other than Ints or UInts, the only values an atomic \
         operation updates"
    );

    site.enter(Stage::Compute).unwrap();
    let invocation = site.builtin(Builtin::GlobalInvocationId);
    let index = site.swizzle(invocation, jess::shader::Swizzle::parse("x").unwrap()).unwrap();
    let zero = site.uint(0);
    let one = site.uint(1);
    let slot = site.atomic(AtomicOp::Add, element(counts, zero, &[]), one, &[zero]).unwrap();
    assert_eq!(slot.ty(), Type::UInt, "an update gives back what the element held");
    site.atomic(AtomicOp::Max, element(counts, one, &[]), index, &[one]).unwrap();
    site.write(element(kept, slot, &[]), index, &[slot]).unwrap();
    let two = site.uint(2);
    let total = site.load(element(counts, zero, &[]), &[zero]).unwrap();
    site.write(element(counts, two, &[]), total, &[two]).unwrap();

    let minus = site.int(-1);
    let refusals = [
        (
            site.atomic(AtomicOp::Add, element(kept, zero, &[]), one, &[zero])
                .unwrap_err(),
            "storage buffer \"kept\" is updated atomically, but it isn't atomic: declare it \
             shader.storage(shader.uint, 'atomic')",
        ),
        (
            site.atomic(AtomicOp::Add, element(counts, zero, &[]), minus, &[zero])
                .unwrap_err(),
            "a UInt in a storage buffer is written a Int",
        ),
    ];
    for (error, message) in refusals {
        assert_eq!(error.to_string(), message);
    }

    let module = wgsl::lower(&builder.finish().unwrap()).unwrap();
    assert_valid_wgsl(&module);
    for expected in [
        "array<atomic<u32>>",
        "= atomicAdd(&b_pass_counts[",
        "= atomicMax(&b_pass_counts[",
        "atomicLoad(&b_pass_counts[",
        "atomicStore(&b_pass_counts[",
    ] {
        assert!(module.source.contains(expected), "{expected} in:\n{}", module.source);
    }
}

#[test]
fn an_atomic_buffer_is_kept_out_of_the_vertex_stage() {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let counts = site
        .storage(Frequency::Pass, "counts", Element::Value(Type::UInt), Access::Atomic)
        .unwrap();
    site.enter(Stage::Vertex).unwrap();
    let zero = site.uint(0);
    let error = site
        .atomic(AtomicOp::Add, element(counts, zero, &[]), zero, &[zero])
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "storage buffer \"counts\" is atomic and used in the vertex function, which may only read storage buffers \
         declared to be read"
    );
}
