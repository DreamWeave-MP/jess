//! The `jess::gpu` integration tests, behind the `gpu` feature. Each runs on whatever adapter the
//! machine has, a software one included, and skips, saying so, on a machine with none.

mod cache;
mod commit;
mod compute;
mod controls;
mod draw;
mod draw_sets;
mod kept;
mod levels;
mod plans;
mod resources;

use jess::gpu::{Gpu, GpuError};

/// The machine's GPU, or `None` after saying the test is skipped.
pub fn gpu(test: &str) -> Option<Gpu> {
    match Gpu::headless() {
        Ok(gpu) => Some(gpu),
        Err(GpuError::NoAdapter) => {
            eprintln!("SKIPPED {test}: no wgpu adapter on this machine, not even a software one");
            None
        }
        Err(error) => panic!("{test}: {error}"),
    }
}

/// Probe, never merged: fails so the log shows which adapter this runner's GPU tests ran on.
#[test]
fn the_adapter_is_reported() {
    let Some(gpu) = gpu("the_adapter_is_reported") else {
        panic!("ADAPTER PROBE: none, so every GPU test on this runner skipped");
    };
    let info = gpu.info();
    panic!(
        "ADAPTER PROBE: {} through {} ({}), indirect first instance {}, base vertex {}",
        info.name,
        info.backend,
        info.kind.name(),
        info.indirect_first_instance,
        info.base_vertex
    );
}
