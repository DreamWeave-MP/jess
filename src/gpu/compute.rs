//! Compute pipelines and dispatches: a compute program run over a grid of workgroups.

use super::bind::{BindGroup, Binding};
use super::pipeline::{Group, invalid_program};
use super::{Gpu, GpuError};
use crate::shader::wgsl::{self, COMPUTE_ENTRY_POINT};
use crate::shader::{Frequency, Program};

/// A compute program ready to dispatch.
#[derive(Debug)]
pub struct ComputePipeline {
    raw: wgpu::ComputePipeline,
    pub(crate) groups: [Group; 3],
    workgroup_size: [u32; 3],
}

impl ComputePipeline {
    /// How many invocations each workgroup runs, as the program declares.
    #[must_use]
    pub fn workgroup_size(&self) -> [u32; 3] {
        self.workgroup_size
    }
}

impl Gpu {
    /// The pipeline that runs `program`, a compute program.
    ///
    /// # Errors
    ///
    /// A vertex and fragment program, or one that doesn't validate.
    pub fn compute_pipeline(&self, program: &Program) -> Result<ComputePipeline, GpuError> {
        let Some(workgroup_size) = program.workgroup_size() else {
            return Err(GpuError::Invalid(
                "a vertex and fragment program draws: make it a render pipeline".to_owned(),
            ));
        };
        let module = wgsl::lower(program).map_err(|errors| invalid_program(&errors))?;
        let groups = self.groups(program, &module.layout.groups, wgpu::ShaderStages::COMPUTE)?;
        let raw = self.checked(|| {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(module.source.as_str().into()),
            });
            let layouts: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(Group::raw_layout).collect();
            let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &layouts,
                immediate_size: 0,
            });
            self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: Some(&layout),
                module: &shader,
                entry_point: Some(COMPUTE_ENTRY_POINT),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        })?;
        Ok(ComputePipeline {
            raw,
            groups,
            workgroup_size,
        })
    }

    /// What `frequency` binds for a compute pipeline, as [`bind_group`](Gpu::bind_group) does for
    /// a render pipeline.
    ///
    /// # Errors
    ///
    /// As [`bind_group`](Gpu::bind_group).
    pub fn compute_bind_group(
        &self,
        pipeline: &ComputePipeline,
        frequency: Frequency,
        bindings: &[Binding<'_>],
    ) -> Result<BindGroup, GpuError> {
        self.bind_group_of(&pipeline.groups[frequency as usize], bindings)
    }

    /// Runs `pipeline` over `workgroups` workgroups on each axis, each of the pipeline's workgroup
    /// size, and submits it.
    ///
    /// # Errors
    ///
    /// A bind group missing for a frequency the program binds, given for one it doesn't, or made
    /// for another pipeline; or no workgroups, or more on an axis than the device allows.
    pub fn dispatch(
        &self,
        pipeline: &ComputePipeline,
        groups: [Option<&BindGroup>; 3],
        workgroups: [u32; 3],
    ) -> Result<(), GpuError> {
        if let Some(problem) = group_problem(pipeline, groups) {
            return Err(GpuError::Invalid(format!("the dispatch is invalid: {problem}")));
        }
        self.check_workgroups(workgroups)?;
        self.checked(|| {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            encode(&mut encoder, pipeline, groups, workgroups);
            self.queue.submit([encoder.finish()]);
        })
    }

    /// Whether the device runs `workgroups` workgroups in one dispatch.
    pub(crate) fn check_workgroups(&self, workgroups: [u32; 3]) -> Result<(), GpuError> {
        let limit = self.info.max_workgroups;
        if workgroups.contains(&0) || workgroups.iter().any(|count| *count > limit) {
            let [x, y, z] = workgroups;
            return Err(GpuError::Invalid(format!(
                "a dispatch of {x}×{y}×{z} workgroups is invalid: expected from 1 to {limit} on each axis"
            )));
        }
        Ok(())
    }
}

/// What is wrong with the bind groups a dispatch of `pipeline` gives, if anything.
pub fn group_problem(pipeline: &ComputePipeline, groups: [Option<&BindGroup>; 3]) -> Option<String> {
    for (frequency, (group, bound)) in Frequency::ALL.into_iter().zip(pipeline.groups.iter().zip(groups)) {
        match (&group.layout, bound) {
            (Some(_), None) => return Some(format!("its program binds a {frequency} group, and the dispatch gives none")),
            (None, Some(_)) => {
                return Some(format!(
                    "its program binds nothing per {frequency}, and the dispatch gives a group"
                ));
            }
            (Some(layout), Some(bound)) if !bound.fits(frequency, layout) => {
                return Some(format!("its {frequency} bind group was made for another frequency or layout"));
            }
            _ => {}
        }
    }
    None
}

/// Records a dispatch whose groups and workgroups were checked into `encoder`.
pub fn encode(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &ComputePipeline,
    groups: [Option<&BindGroup>; 3],
    workgroups: [u32; 3],
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
    pass.set_pipeline(&pipeline.raw);
    for (group, bound) in (0..).zip(groups) {
        if let Some(bound) = bound {
            pass.set_bind_group(group, &bound.raw, &[]);
        }
    }
    let [x, y, z] = workgroups;
    pass.dispatch_workgroups(x, y, z);
}
