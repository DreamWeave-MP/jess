//! The pipeline cache: a device compiles one pipeline per distinct lowered program and state, and
//! hands the same one to everyone who asks again while anyone still holds it.
//!
//! The key is the WGSL a program lowers to, which is deterministic and holds every binding and
//! attribute the pipeline's layouts come from, plus the render state. A cached pipeline is held
//! weakly: once nothing uses it, the next request compiles it again.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, Weak};

use super::compute::ComputePipeline;
use super::pipeline::invalid_program;
use super::{Gpu, GpuError, PipelineState, RenderPipeline};
use crate::shader::{Program, wgsl};

/// How often a device's pipeline cache compiled and how often it handed back one it had.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PipelineStats {
    /// Pipelines compiled.
    pub compiled: u64,
    /// Requests answered with a pipeline already compiled.
    pub reused: u64,
}

/// What a device's cache holds.
#[derive(Debug, Default)]
pub struct Cache {
    render: HashMap<(String, PipelineState), Weak<RenderPipeline>>,
    compute: HashMap<String, Weak<ComputePipeline>>,
    stats: PipelineStats,
}

impl Gpu {
    /// [`render_pipeline`](Gpu::render_pipeline), shared: the pipeline compiled for a program
    /// that lowers to the same WGSL with the same state, while anything holds it.
    ///
    /// # Errors
    ///
    /// As [`render_pipeline`](Gpu::render_pipeline).
    pub fn cached_render_pipeline(&self, program: &Program, state: &PipelineState) -> Result<Arc<RenderPipeline>, GpuError> {
        if program.workgroup_size().is_some() {
            return self.render_pipeline(program, state).map(Arc::new);
        }
        let module = wgsl::lower(program).map_err(|errors| invalid_program(&errors))?;
        let key = (module.source.clone(), state.clone());
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pipeline) = cache.render.get(&key).and_then(Weak::upgrade) {
            cache.stats.reused += 1;
            return Ok(pipeline);
        }
        let pipeline = Arc::new(self.render_pipeline_of(program, &module, state)?);
        cache.render.retain(|_, held| held.strong_count() > 0);
        cache.render.insert(key, Arc::downgrade(&pipeline));
        cache.stats.compiled += 1;
        drop(cache);
        Ok(pipeline)
    }

    /// [`compute_pipeline`](Gpu::compute_pipeline), shared as
    /// [`cached_render_pipeline`](Gpu::cached_render_pipeline) shares render pipelines.
    ///
    /// # Errors
    ///
    /// As [`compute_pipeline`](Gpu::compute_pipeline).
    pub fn cached_compute_pipeline(&self, program: &Program) -> Result<Arc<ComputePipeline>, GpuError> {
        let Some(workgroup_size) = program.workgroup_size() else {
            return self.compute_pipeline(program).map(Arc::new);
        };
        let module = wgsl::lower(program).map_err(|errors| invalid_program(&errors))?;
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(pipeline) = cache.compute.get(&module.source).and_then(Weak::upgrade) {
            cache.stats.reused += 1;
            return Ok(pipeline);
        }
        let pipeline = Arc::new(self.compute_pipeline_of(program, &module, workgroup_size)?);
        cache.compute.retain(|_, held| held.strong_count() > 0);
        cache.compute.insert(module.source, Arc::downgrade(&pipeline));
        cache.stats.compiled += 1;
        drop(cache);
        Ok(pipeline)
    }

    /// How often the cache compiled and reused pipelines since the device opened.
    #[must_use]
    pub fn pipeline_stats(&self) -> PipelineStats {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner).stats
    }
}
