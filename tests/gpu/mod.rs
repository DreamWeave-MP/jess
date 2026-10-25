//! The `jess::gpu` integration tests, behind the `gpu` feature. Each runs on whatever adapter the
//! machine has, a software one included, and skips, saying so, on a machine with none.

mod compute;
mod draw;
mod draw_sets;
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
