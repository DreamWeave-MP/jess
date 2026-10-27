//! Kept resources: what a plan holds from run to run, read back from the plan, at the size its
//! viewport makes them.

use super::gpu;
use super::plans::{VIEWPORT, brighten};

use jess::gpu::{DrawSets, PlanBinding, PlanGroup, Work};
use jess::render::{
    BufferSize, ClearValue, Extent, LoadOp, Pass, PassKind, PassMetadata, PassOutput, ReadUsage, RenderGraph, ResourceDesc,
    ResourceOrigin, StoreOp, TargetSize, TextureFormat,
};

fn pass(kind: PassKind, name: &str) -> Pass {
    Pass::new(kind, PassMetadata::new(name, "@test/kept"))
}

#[test]
fn a_kept_buffer_carries_each_run_into_the_next() {
    let Some(gpu) = gpu("a_kept_buffer_carries_each_run_into_the_next") else {
        return;
    };
    let mut graph = RenderGraph::new("accumulate");
    let colors = graph
        .add("colors", ResourceDesc::Buffer(BufferSize::Bytes(64)), ResourceOrigin::Kept)
        .unwrap();
    graph
        .add_pass(pass(PassKind::Compute, "brighten").write(PassOutput::storage(colors, LoadOp::Load, StoreOp::Store)))
        .unwrap();
    let (pipeline, storage) = brighten(&gpu);
    let work = vec![Work::Compute {
        pipeline,
        groups: [
            Some(PlanGroup::Make(vec![PlanBinding::Buffer(storage, "colors".to_owned())])),
            None,
            None,
        ],
        workgroups: [1, 1, 1],
    }];
    let plan = gpu.plan(&graph, work, VIEWPORT).unwrap();
    let floats = |bytes: Vec<u8>| -> Vec<f32> {
        bytes
            .chunks(4)
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect()
    };
    assert_eq!(
        floats(gpu.read_kept(&plan, "colors", 0, 0).unwrap()),
        [0.0; 16],
        "zeroed at first"
    );
    for _ in 0..3 {
        gpu.run(&plan, &DrawSets::new(), &[]).unwrap();
    }
    let kept = floats(gpu.read_kept(&plan, "colors", 0, 0).unwrap());
    assert!(kept.iter().all(|value| (value - 0.6).abs() < 1e-5), "{kept:?}");
    assert_eq!(
        gpu.read_kept(&plan, "colors", 1, 0).unwrap_err().to_string(),
        "colors is a buffer, read whole: it has no level 1 of layer 0"
    );
}

#[test]
fn a_kept_buffer_sized_by_the_viewport_holds_a_frame() {
    let Some(gpu) = gpu("a_kept_buffer_sized_by_the_viewport_holds_a_frame") else {
        return;
    };
    let mut graph = RenderGraph::new("readback");
    let color = graph
        .add_render_target("color", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let image = BufferSize::Image {
        format: TextureFormat::Rgba8Unorm,
        scale: 1.0,
    };
    let pixels = graph
        .add("pixels", ResourceDesc::Buffer(image), ResourceOrigin::Kept)
        .unwrap();
    let orange = LoadOp::Clear(ClearValue::Color([1.0, 0.5, 0.0, 1.0]));
    graph
        .add_pass(pass(PassKind::Mesh, "clear").write(PassOutput::color(color, orange, StoreOp::Store)))
        .unwrap();
    graph
        .add_pass(
            pass(PassKind::Copy, "readBack")
                .read(color, ReadUsage::Copy)
                .write(PassOutput::copy(pixels, LoadOp::DontCare, StoreOp::Store)),
        )
        .unwrap();
    let work = vec![
        Work::Draw {
            tags: Vec::new(),
            pipeline: None,
            pass: None,
        },
        Work::Copy,
    ];
    let plan = gpu.plan(&graph, work, VIEWPORT).unwrap();
    assert_eq!(plan.buffer_bytes("pixels").unwrap(), 256 * 16);
    gpu.run(&plan, &DrawSets::new(), &[]).unwrap();
    let read = gpu.read_kept(&plan, "pixels", 0, 0).unwrap();
    assert_eq!(read.len(), 256 * 16);
    assert_eq!(read[256 * 15..256 * 15 + 4], [255, 128, 0, 255]);

    let small = plan.resized(&gpu, Extent { width: 8, height: 4 }).unwrap();
    assert_eq!(small.buffer_bytes("pixels").unwrap(), 256 * 4);
    assert_eq!(small.target_extent("color").unwrap(), Extent { width: 8, height: 4 });
    assert_eq!(
        gpu.read_kept(&plan, "color", 0, 0).unwrap_err().to_string(),
        "color is transient, not kept: only the plan's kept resources are read from it"
    );
}
