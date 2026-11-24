//! The pipeline cache: the same program and state compile once, while anything holds the result.

use std::sync::Arc;

use super::gpu;

use jess::gpu::{ColorState, Compare, DepthBias, DepthState, PipelineState, PipelineStats};
use jess::render::TextureFormat;
use jess::shader::{Program, ProgramBuilder, Type};

/// A program drawing one point in `red`.
fn point(red: f32) -> Program {
    let mut builder = ProgramBuilder::new();
    let mut site = builder.site();
    let zero = site.float(0.0).unwrap();
    let one = site.float(1.0).unwrap();
    let position = site.construct(Type::Vec4f, &[zero, zero, zero, one]).unwrap();
    site.set_position(position).unwrap();
    let red = site.float(red).unwrap();
    let color = site.construct(Type::Vec4f, &[red, zero, zero, one]).unwrap();
    site.output(color).unwrap();
    builder.finish().unwrap()
}

#[test]
fn the_same_program_and_state_compile_once() {
    let Some(gpu) = gpu("the_same_program_and_state_compile_once") else {
        return;
    };
    let state = |format| PipelineState {
        colors: vec![ColorState::new(format)],
        ..PipelineState::default()
    };
    let rgba = state(TextureFormat::Rgba8Unorm);
    let first = gpu.cached_render_pipeline(&point(1.0), &rgba).unwrap();
    let again = gpu.cached_render_pipeline(&point(1.0), &rgba).unwrap();
    assert!(Arc::ptr_eq(&first, &again), "a program built twice lowers alike");
    assert_eq!(gpu.pipeline_stats(), PipelineStats { compiled: 1, reused: 1 });

    let other_state = gpu
        .cached_render_pipeline(&point(1.0), &state(TextureFormat::Rgba16Float))
        .unwrap();
    let other_program = gpu.cached_render_pipeline(&point(0.5), &rgba).unwrap();
    assert!(!Arc::ptr_eq(&first, &other_state) && !Arc::ptr_eq(&first, &other_program));
    assert_eq!(gpu.pipeline_stats(), PipelineStats { compiled: 3, reused: 1 });

    drop((first, again));
    gpu.cached_render_pipeline(&point(1.0), &rgba).unwrap();
    assert_eq!(
        gpu.pipeline_stats(),
        PipelineStats { compiled: 4, reused: 1 },
        "nothing held it, so it was compiled again"
    );
}

/// Coverage and bias are state like the rest: pipelines that differ only there are two, and a
/// bias of the same numbers is the same pipeline, a zero of either sign being zero.
#[test]
fn coverage_and_bias_tell_pipelines_apart() {
    let Some(gpu) = gpu("coverage_and_bias_tell_pipelines_apart") else {
        return;
    };
    let state = |alpha_to_coverage, slope_scale| PipelineState {
        colors: vec![ColorState::new(TextureFormat::Rgba8Unorm)],
        depth: Some(DepthState {
            format: TextureFormat::Depth32Float,
            compare: Compare::Less,
            write: true,
            bias: DepthBias {
                slope_scale,
                ..DepthBias::default()
            },
        }),
        multisampled: true,
        alpha_to_coverage,
        ..PipelineState::default()
    };
    let program = point(1.0);
    let plain = gpu.cached_render_pipeline(&program, &state(false, 0.0)).unwrap();
    let covering = gpu.cached_render_pipeline(&program, &state(true, 0.0)).unwrap();
    let biased = gpu.cached_render_pipeline(&program, &state(false, 1.5)).unwrap();
    assert!(!Arc::ptr_eq(&plain, &covering) && !Arc::ptr_eq(&plain, &biased));
    assert_eq!(gpu.pipeline_stats(), PipelineStats { compiled: 3, reused: 0 });

    let again = gpu.cached_render_pipeline(&program, &state(false, 1.5)).unwrap();
    let negative = gpu.cached_render_pipeline(&program, &state(false, -0.0)).unwrap();
    assert!(Arc::ptr_eq(&biased, &again) && Arc::ptr_eq(&plain, &negative));
    assert_eq!(gpu.pipeline_stats(), PipelineStats { compiled: 3, reused: 2 });
}
