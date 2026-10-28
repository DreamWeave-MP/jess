//! What a script changes about a plan between runs: passes left out and put back, the region a
//! copy pass copies, and the collection of draw sets a run draws from; and that a run makes
//! nothing it made the run before.

use std::sync::Arc;

use super::gpu;
use super::plans::{ROW, SIZE, VIEWPORT, copied, output, run_scene, scene, swapped};

use jess::gpu::{BufferUsage, DrawSets, Import, PlanBinding, PlanGroup, TextureDesc, TextureUsage, Work};
use jess::render::{
    CopyAt, CopyRegion, CopySize, Extent, LoadOp, Pass, PassKind, PassMetadata, PassOutput, ReadUsage, RenderGraph, StoreOp,
    TargetSize, TextureFormat,
};

#[test]
fn passes_are_left_out_and_put_back() {
    let Some(gpu) = gpu("passes_are_left_out_and_put_back") else {
        return;
    };
    let scene = scene(&gpu);
    let plan = gpu.plan(&scene.graph, scene.work.clone(), VIEWPORT).unwrap();

    // The present pass samples the color the overlay stores last, so the overlay goes with it.
    assert_eq!(
        plan.set_enabled(&["overlay"], false).unwrap_err().to_string(),
        "pass present uses what pass overlay stores: leave both out, or neither"
    );
    assert!(plan.enabled("overlay").unwrap(), "a refused change changes nothing");
    assert_eq!(
        plan.set_enabled(&["debug"], false).unwrap_err().to_string(),
        "pass debug is culled: nothing sees what it does, so it never runs"
    );
    assert!(plan.set_enabled(&["nowhere"], false).is_err());

    // Left out, the copy leaves the pixels as they were.
    plan.set_enabled(&["readBack"], false).unwrap();
    assert!(!plan.enabled("readBack").unwrap());
    let target = output(&gpu);
    let pixels = gpu.buffer(u64::from(ROW * SIZE), BufferUsage::default()).unwrap();
    gpu.write_buffer(&pixels, 0, &vec![7; (ROW * SIZE) as usize]).unwrap();
    let imports = [Import::Texture("output", &target), Import::Buffer("pixels", &pixels)];
    gpu.run(&plan, &scene.sets, &imports).unwrap();
    assert_eq!(gpu.read_buffer(&pixels, 0, 8).unwrap(), [7; 8]);

    plan.set_enabled(&["overlay", "present", "readBack"], true).unwrap();
    gpu.run(&plan, &scene.sets, &imports).unwrap();
    assert_eq!(
        copied(&gpu.read_buffer(&pixels, 0, pixels.size()).unwrap(), 2, 8),
        [0, 153, 0, 255]
    );
}

#[test]
fn a_copy_region_changes_between_runs_and_follows_resizes() {
    let Some(gpu) = gpu("a_copy_region_changes_between_runs_and_follows_resizes") else {
        return;
    };
    let scene = scene(&gpu);
    let plan = gpu.plan(&scene.graph, scene.work.clone(), VIEWPORT).unwrap();

    // One texel, the way a picking pass reads the one under the pointer.
    let texel = |x, y| CopyRegion {
        from: Some(CopyAt::Texel { x, y }),
        to: None,
        size: Some(CopySize::Texels { width: 1, height: 1 }),
    };
    plan.set_region("readBack", texel(SIZE - 2, 8)).unwrap();
    let pixels = run_scene(&gpu, &plan, &scene.sets);
    assert_eq!(copied(&pixels, 0, 0), [153, 0, 0, 255]);
    assert_eq!(copied(&pixels, 1, 0), [0; 4], "nothing past the one texel");

    assert_eq!(
        plan.set_region("readBack", texel(SIZE, 0)).unwrap_err().to_string(),
        "pass readBack is invalid: 1×1 texels at 16, 0 run past level 0 of layer 0 of \"output\", 16×16"
    );
    assert_eq!(
        plan.set_region("present", texel(0, 0)).unwrap_err().to_string(),
        "pass present isn't a copy, so it has no region"
    );

    // A plan resized from it copies the same region, at its own size, and shares its passes.
    let small = Extent { width: 8, height: 8 };
    assert!(plan.resized(&gpu, small).is_err(), "texel (14, 8) is outside 8×8");
    plan.set_region("readBack", texel(2, 2)).unwrap();
    let resized = plan.resized(&gpu, small).unwrap();
    resized.set_enabled(&["readBack"], false).unwrap();
    assert!(!plan.enabled("readBack").unwrap());
}

#[test]
fn runs_draw_from_the_collection_they_are_given() {
    let Some(gpu) = gpu("runs_draw_from_the_collection_they_are_given") else {
        return;
    };
    let scene = scene(&gpu);
    let plan = gpu.plan(&scene.graph, scene.work.clone(), VIEWPORT).unwrap();
    let opaque = scene.sets.tagged(&["opaque".to_owned()]);
    let (green, red) = (&opaque[0], &opaque[1]);

    // Only the red rectangle, tinted and swapped: blue everywhere.
    let other = DrawSets::new();
    other.add(red);
    assert_eq!(copied(&run_scene(&gpu, &plan, &other), 2, 8), [0, 0, 153, 255]);

    // A set added between runs is drawn by the next.
    other.add(green);
    assert_eq!(copied(&run_scene(&gpu, &plan, &other), 2, 8), [0, 153, 0, 255]);
    assert_eq!(copied(&run_scene(&gpu, &plan, &scene.sets), SIZE - 2, 8), [153, 0, 0, 255]);
}

#[test]
fn a_run_makes_nothing_the_run_before_made() {
    let Some(gpu) = gpu("a_run_makes_nothing_the_run_before_made") else {
        return;
    };
    // A fullscreen pass sampling an imported texture: its group binds the import, so a run makes
    // it, and the next run with the same texture keeps it.
    let mut graph = RenderGraph::new("imported");
    let source = graph
        .import_render_target("source", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    let screen = graph
        .import_render_target("screen", TextureFormat::Rgba8Unorm, TargetSize::VIEWPORT)
        .unwrap();
    graph
        .add_pass(
            Pass::new(PassKind::Fullscreen, PassMetadata::new("present", "@test/controls"))
                .read(source, ReadUsage::Texture)
                .write(PassOutput::color(screen, LoadOp::DontCare, StoreOp::Store)),
        )
        .unwrap();
    let (present, binding) = swapped(&gpu);
    let work = vec![Work::Fullscreen {
        pipeline: present,
        groups: [
            None,
            Some(PlanGroup::Make(vec![PlanBinding::Target(binding, "source".to_owned())])),
            None,
        ],
    }];
    let plan = gpu.plan(&graph, work, VIEWPORT).unwrap();
    let texture = || {
        let usage = TextureUsage::SAMPLED | TextureUsage::TARGET;
        Arc::new(
            gpu.texture(&TextureDesc::d2(SIZE, SIZE, TextureFormat::Rgba8Unorm, usage))
                .unwrap(),
        )
    };
    let (first, second, target) = (texture(), texture(), output(&gpu));
    let sets = DrawSets::new();
    let run = |given: &jess::gpu::Texture| {
        gpu.run(
            &plan,
            &sets,
            &[Import::Texture("source", given), Import::Texture("screen", &target)],
        )
        .unwrap();
    };

    run(&first);
    let made = gpu.made();
    for _ in 0..3 {
        run(&first);
    }
    assert_eq!(gpu.made(), made, "the same import binds the group made for it");
    run(&second);
    assert_eq!(gpu.made().bind_groups, made.bind_groups + 1, "another import makes it again");
}
