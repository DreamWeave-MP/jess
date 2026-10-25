//! Plans: a render graph compiled for a device and a viewport, with what each of its passes does,
//! run whole, every frame, without being built again.
//!
//! [`Gpu::plan`] compiles the graph (its order, the passes nothing observes, which transient
//! resources share memory), allocates one texture or buffer for each group of transient resources
//! that share, and makes every bind group it can. [`Gpu::run`] then records every surviving pass in
//! order into one submission: a draw pass draws the visible draw sets carrying its tags, a
//! fullscreen pass draws one triangle over its targets, a compute pass dispatches its pipeline, and
//! a copy pass copies its source into its destination. Imported targets and buffers, a window's
//! backbuffer or a buffer read back afterwards, are given to each run.

use std::sync::Arc;

use super::bind::{BindGroup, Binding};
use super::compute::{self, ComputePipeline};
use super::draw_set::{DrawSet, DrawSets};
use super::pass::{check_pass, encode};
use super::pipeline::Slot;
use super::texture::{TextureDesc, TextureUsage};
use super::{Buffer, BufferUsage, Draw, Gpu, GpuError, Group, RenderPipeline, Sampler, Target, Texture};
use crate::render::{
    AliasGroup, Attachment, Extent, LoadOp, Pass, PassKind, ReadUsage, RenderGraph, ResourceDesc, ResourceOrigin, StoreOp,
    TextureFormat,
};
use crate::shader::{Access, Frequency, ResourceId, StorageId};

/// What one pass of a plan's graph does.
#[derive(Clone, Debug)]
pub enum Work {
    /// Draws every visible draw set carrying any of `tags`, in the order they were added, with
    /// `pass` as their pass group. A mesh pass's work.
    Draw {
        /// What the sets it draws carry.
        tags: Vec<String>,
        /// The pipeline whose pass group layout a [`PlanGroup::Make`] pass group is made for:
        /// any pipeline whose program declares the pass group the sets bind.
        pipeline: Option<Arc<RenderPipeline>>,
        /// The pass group every set is drawn with, if their programs bind one.
        pass: Option<PlanGroup>,
    },
    /// Draws three vertices, no buffers, with `pipeline`: one triangle over the targets, for a
    /// program that places it from the vertex index. A fullscreen pass's work.
    Fullscreen {
        /// The pipeline.
        pipeline: Arc<RenderPipeline>,
        /// Its pass, material, and draw groups, for each its program binds anything for.
        groups: [Option<PlanGroup>; 3],
    },
    /// Runs `pipeline` over `workgroups` workgroups on each axis. A compute pass's work.
    Compute {
        /// The pipeline.
        pipeline: Arc<ComputePipeline>,
        /// Its pass, material, and draw groups, for each its program binds anything for.
        groups: [Option<PlanGroup>; 3],
        /// How many workgroups on each axis.
        workgroups: [u32; 3],
    },
    /// Copies the pass's source into its destination, which the graph names. A copy pass's work.
    Copy,
}

/// A bind group a plan draws or dispatches with.
#[derive(Clone, Debug)]
pub enum PlanGroup {
    /// One made already.
    Bound(Arc<BindGroup>),
    /// One the plan makes, for its pass's pipeline's layout of the frequency it is given for, from
    /// resources and the graph's own targets and buffers.
    Make(Vec<PlanBinding>),
}

/// One thing a group a plan makes binds.
#[derive(Clone, Debug)]
pub enum PlanBinding {
    /// The group's uniform block.
    Uniform(Arc<Buffer>),
    /// A declared texture.
    Texture(ResourceId, Arc<Texture>),
    /// A declared sampler.
    Sampler(ResourceId, Arc<Sampler>),
    /// A declared storage buffer.
    Storage(StorageId, Arc<Buffer>),
    /// A declared texture, bound to the graph's target of this name, which the pass reads.
    Target(ResourceId, String),
    /// A declared storage buffer, bound to the graph's buffer of this name: one the pass reads,
    /// for a read-only declaration, or writes, for a read-write one.
    Buffer(StorageId, String),
}

/// A texture or buffer given to a run for one of the graph's imported resources, by name.
#[derive(Clone, Copy, Debug)]
pub enum Import<'a> {
    /// A texture, for an imported target.
    Texture(&'a str, &'a Texture),
    /// A buffer, for an imported buffer.
    Buffer(&'a str, &'a Buffer),
}

impl Import<'_> {
    fn name(&self) -> &str {
        match self {
            Import::Texture(name, _) | Import::Buffer(name, _) => name,
        }
    }
}

/// A resource of the graph as one run sees it.
#[derive(Clone, Copy, Debug)]
enum Live<'a> {
    Texture(&'a Texture),
    Buffer(&'a Buffer),
}

/// A transient resource's allocation.
#[derive(Clone, Debug)]
enum Allocation {
    Texture(Arc<Texture>),
    Buffer(Arc<Buffer>),
}

impl Allocation {
    fn live(&self) -> Live<'_> {
        match self {
            Allocation::Texture(texture) => Live::Texture(texture),
            Allocation::Buffer(buffer) => Live::Buffer(buffer),
        }
    }
}

/// The pipeline a group is laid out for.
#[derive(Clone, Debug)]
enum Owner {
    Render(Arc<RenderPipeline>),
    Compute(Arc<ComputePipeline>),
}

impl Owner {
    fn group(&self, frequency: Frequency) -> &Group {
        match self {
            Owner::Render(pipeline) => &pipeline.groups[frequency as usize],
            Owner::Compute(pipeline) => &pipeline.groups[frequency as usize],
        }
    }
}

/// A group a plan draws or dispatches with, as it was made.
#[derive(Debug)]
enum Planned {
    /// Made already, by the caller or the plan.
    Made(Arc<BindGroup>),
    /// Made for each run, because it binds an imported resource.
    EachRun {
        owner: Owner,
        frequency: Frequency,
        bindings: Vec<PlanBinding>,
    },
}

/// What a surviving pass does, as it was planned.
#[derive(Debug)]
enum PlannedWork {
    Draw {
        tags: Vec<String>,
        pass: Option<Planned>,
    },
    Fullscreen {
        pipeline: Arc<RenderPipeline>,
        groups: [Option<Planned>; 3],
    },
    Compute {
        pipeline: Arc<ComputePipeline>,
        groups: [Option<Planned>; 3],
        workgroups: [u32; 3],
    },
    Copy,
}

/// A pass that survived, ready to record.
#[derive(Debug)]
struct PlannedPass {
    /// Its index in the graph.
    index: usize,
    work: PlannedWork,
}

/// A render graph compiled for a device and a viewport, ready to run.
#[derive(Debug)]
pub struct Plan {
    graph: RenderGraph,
    work: Vec<Work>,
    viewport: Extent,
    /// Each of the graph's resources' allocations: a transient one's, shared with those it
    /// aliases; `None` for an imported one, and for one no surviving pass uses.
    allocations: Vec<Option<Allocation>>,
    passes: Vec<PlannedPass>,
    /// Where each surviving pass's timestamps go, on a device that times passes.
    timing: Option<Timing>,
}

/// A plan's timestamps: a pair of queries for each drawing or compute pass, and the buffer they
/// resolve into at the end of every run.
#[derive(Debug)]
struct Timing {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    /// Each surviving pass's first query, in order; `None` for a copy, which isn't timed.
    first: Vec<Option<u32>>,
    count: u32,
}

/// How long one pass of a plan's last run took on the GPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassTiming {
    /// The pass.
    pub pass: String,
    /// How long it took, from its first command to its last; `None` for a copy pass, which isn't
    /// timed.
    pub duration: Option<std::time::Duration>,
}

impl Plan {
    /// The graph it was compiled from.
    #[must_use]
    pub fn graph(&self) -> &RenderGraph {
        &self.graph
    }

    /// The viewport its relative targets are sized for.
    #[must_use]
    pub fn viewport(&self) -> Extent {
        self.viewport
    }

    /// The graph's passes that survived, in the order they run, by name.
    #[must_use]
    pub fn order(&self) -> Vec<&str> {
        self.passes
            .iter()
            .map(|pass| self.graph.passes()[pass.index].name())
            .collect()
    }

    /// How many textures and buffers it allocated for the graph's transient resources: one for
    /// each group of them that share memory.
    #[must_use]
    pub fn allocations(&self) -> usize {
        let mut seen: Vec<*const ()> = Vec::new();
        for allocation in self.allocations.iter().flatten() {
            let pointer = match allocation {
                Allocation::Texture(texture) => Arc::as_ptr(texture).cast::<()>(),
                Allocation::Buffer(buffer) => Arc::as_ptr(buffer).cast::<()>(),
            };
            if !seen.contains(&pointer) {
                seen.push(pointer);
            }
        }
        seen.len()
    }

    /// The same plan for `viewport`: its transient targets sized again and the groups that read
    /// them made again. Pipelines are kept; nothing is compiled.
    ///
    /// # Errors
    ///
    /// What [`Gpu::plan`] refuses, which for a plan that compiled is only what the device
    /// refuses, and copies between targets and buffers that no longer fit.
    pub fn resized(&self, gpu: &Gpu, viewport: Extent) -> Result<Plan, GpuError> {
        gpu.plan(&self.graph, self.work.clone(), viewport)
    }

    /// Every resource the run sees: the plan's own and `imports`, each checked against the
    /// graph's declaration.
    fn with_imports<'a>(&'a self, imports: &[Import<'a>]) -> Result<Vec<Option<Live<'a>>>, GpuError> {
        let graph = &self.graph;
        let mut live: Vec<Option<Live<'a>>> = self
            .allocations
            .iter()
            .map(|allocation| allocation.as_ref().map(Allocation::live))
            .collect();
        for import in imports {
            let name = import.name();
            let Some(id) = graph.resource_id(name) else {
                return Err(GpuError::Invalid(format!(
                    "the run gives {name}, which the graph has no target or buffer called"
                )));
            };
            let resource = &graph.resources()[id.index()];
            if resource.origin != ResourceOrigin::Imported {
                return Err(GpuError::Invalid(format!(
                    "the run gives {name}, which is the plan's own, not an imported one"
                )));
            }
            let given = match (*import, resource.desc) {
                (Import::Texture(_, texture), ResourceDesc::Texture { format, size }) => {
                    let desc = texture.desc();
                    let size = size.resolve(self.viewport);
                    let given = Extent {
                        width: desc.width,
                        height: desc.height,
                    };
                    if desc.format != format || given != size {
                        return Err(GpuError::Invalid(format!(
                            "the run gives {name} a {}×{} {} texture: expected {}×{} {format}",
                            given.width, given.height, desc.format, size.width, size.height
                        )));
                    }
                    Live::Texture(texture)
                }
                (Import::Buffer(_, buffer), ResourceDesc::Buffer { bytes }) => {
                    if buffer.size() != bytes {
                        return Err(GpuError::Invalid(format!(
                            "the run gives {name} a {}-byte buffer: expected {bytes} bytes",
                            buffer.size()
                        )));
                    }
                    Live::Buffer(buffer)
                }
                (Import::Texture(..), ResourceDesc::Buffer { .. }) => {
                    return Err(GpuError::Invalid(format!(
                        "the run gives {name} a texture, and it is a buffer"
                    )));
                }
                (Import::Buffer(..), ResourceDesc::Texture { .. }) => {
                    return Err(GpuError::Invalid(format!(
                        "the run gives {name} a buffer, and it is a target"
                    )));
                }
            };
            if live[id.index()].replace(given).is_some() {
                return Err(GpuError::Invalid(format!("the run gives {name} twice")));
            }
        }
        Ok(live)
    }
}

/// `problem`, in pass `name`.
fn in_pass(name: &str, problem: impl std::fmt::Display) -> GpuError {
    GpuError::Invalid(format!("pass {name} is invalid: {problem}"))
}

/// The bytes one row of a target takes in a buffer it is copied to or from: its texels, padded to
/// a multiple of 256 bytes.
fn copy_row(format: TextureFormat, width: u32) -> Option<u32> {
    Some((format.texel_bytes()? * width).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT))
}

impl Gpu {
    /// Compiles `graph` with `work`, one for each of its passes in the order they were added,
    /// for a viewport of `viewport` pixels.
    ///
    /// # Errors
    ///
    /// A graph that doesn't compile; work missing or extra, or the wrong kind for its pass (draw
    /// sets for a mesh pass, a fullscreen pipeline for a fullscreen pass, a compute pipeline for a
    /// compute pass, nothing for a copy); a fullscreen pipeline whose targets aren't its pass's; a
    /// dispatch of more workgroups than the device runs; a copy between a target and a buffer too
    /// small for it, or into a depth target from a buffer; a group that binds a target or buffer
    /// its pass doesn't use as its declaration does, or that doesn't fit its pipeline; a resource
    /// the device can't allocate.
    pub fn plan(&self, graph: &RenderGraph, work: Vec<Work>, viewport: Extent) -> Result<Plan, GpuError> {
        let compiled = graph
            .compile()
            .map_err(|error| GpuError::Invalid(format!("the render graph doesn't compile: {error}")))?;
        if work.len() != graph.passes().len() {
            return Err(GpuError::Invalid(format!(
                "the render graph has {} passes, and the plan has work for {}: expected one each",
                graph.passes().len(),
                work.len()
            )));
        }
        let allocations = self.allocate(graph, compiled.alias_groups(), viewport)?;
        let mut passes = Vec::with_capacity(compiled.order().len());
        for id in compiled.order() {
            let index = id.index();
            let pass = &graph.passes()[index];
            let planned = self
                .planned_work(graph, &allocations, pass, &work[index], viewport)
                .map_err(|problem| in_pass(pass.name(), problem))?;
            passes.push(PlannedPass { index, work: planned });
        }
        let timing = self.timing(&passes)?;
        Ok(Plan {
            graph: graph.clone(),
            work,
            viewport,
            allocations,
            passes,
            timing,
        })
    }

    /// The queries `passes` are timed with, on a device that times passes.
    fn timing(&self, passes: &[PlannedPass]) -> Result<Option<Timing>, GpuError> {
        if !self.info.timestamps {
            return Ok(None);
        }
        let mut count = 0;
        let first: Vec<Option<u32>> = passes
            .iter()
            .map(|pass| {
                if matches!(pass.work, PlannedWork::Copy) {
                    return None;
                }
                count += 2;
                Some(count - 2)
            })
            .collect();
        if count == 0 {
            return Ok(None);
        }
        self.checked(|| Timing {
            set: self.device.create_query_set(&wgpu::QuerySetDescriptor {
                label: None,
                ty: wgpu::QueryType::Timestamp,
                count,
            }),
            resolve: self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: u64::from(count) * 8,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            first,
            count,
        })
        .map(Some)
    }

    /// How long each pass of `plan`'s last run took on the GPU, in the order they ran; `None` on a
    /// device that doesn't time passes. A plan that hasn't run reports zeros. Reading them waits
    /// for that run to finish.
    ///
    /// # Errors
    ///
    /// What the device refuses in reading them back.
    pub fn timings(&self, plan: &Plan) -> Result<Option<Vec<PassTiming>>, GpuError> {
        let Some(timing) = &plan.timing else {
            return Ok(None);
        };
        let size = u64::from(timing.count) * 8;
        let bytes = self.read_back(size, |encoder, staging| {
            encoder.copy_buffer_to_buffer(&timing.resolve, 0, staging, 0, size);
        })?;
        let stamp = |index: u32| {
            let at = index as usize * 8;
            u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8]))
        };
        let period = f64::from(self.queue.get_timestamp_period());
        Ok(Some(
            plan.passes
                .iter()
                .zip(&timing.first)
                .map(|(pass, first)| PassTiming {
                    pass: plan.graph.passes()[pass.index].name().to_owned(),
                    // Ticks between two stamps of one pass are far below 2^53.
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    duration: first.map(|first| {
                        let ticks = stamp(first + 1).saturating_sub(stamp(first));
                        std::time::Duration::from_nanos((ticks as f64 * period) as u64)
                    }),
                })
                .collect(),
        ))
    }

    /// A texture or buffer for each group of transient resources that share one, a texture at its
    /// size for `viewport`.
    fn allocate(
        &self,
        graph: &RenderGraph,
        groups: &[AliasGroup],
        viewport: Extent,
    ) -> Result<Vec<Option<Allocation>>, GpuError> {
        let mut allocations = vec![None; graph.resources().len()];
        for group in groups {
            let first = &graph.resources()[group.resources[0].index()];
            let allocation = match first.desc {
                ResourceDesc::Texture { format, size } => {
                    let size = size.resolve(viewport);
                    let usage = TextureUsage::TARGET | TextureUsage::SAMPLED;
                    self.texture(&TextureDesc::d2(size.width, size.height, format, usage))
                        .map(|texture| Allocation::Texture(Arc::new(texture)))
                }
                ResourceDesc::Buffer { bytes } => {
                    let usage = BufferUsage::VERTEX | BufferUsage::INDEX | BufferUsage::UNIFORM | BufferUsage::STORAGE;
                    self.buffer(bytes, usage).map(|buffer| Allocation::Buffer(Arc::new(buffer)))
                }
            }
            .map_err(|problem| GpuError::Invalid(format!("{} can't be made: {problem}", first.name)))?;
            for member in &group.resources {
                allocations[member.index()] = Some(allocation.clone());
            }
        }
        Ok(allocations)
    }

    /// `work` for `pass`, checked against it, with every group it can make made.
    fn planned_work(
        &self,
        graph: &RenderGraph,
        allocations: &[Option<Allocation>],
        pass: &Pass,
        work: &Work,
        viewport: Extent,
    ) -> Result<PlannedWork, String> {
        let make = |owner: &Owner, frequency: Frequency, group: &PlanGroup| {
            self.planned(graph, allocations, pass, owner, frequency, group)
        };
        let make_all = |owner: Owner, groups: &[Option<PlanGroup>; 3]| -> Result<[Option<Planned>; 3], String> {
            let mut planned = [None, None, None];
            for (frequency, group) in Frequency::ALL.into_iter().zip(groups) {
                planned[frequency as usize] = group.as_ref().map(|group| make(&owner, frequency, group)).transpose()?;
            }
            Ok(planned)
        };
        match (work, pass.kind) {
            (
                Work::Draw {
                    tags,
                    pipeline,
                    pass: group,
                },
                PassKind::Mesh,
            ) => {
                let pass = match (group, pipeline) {
                    (None, _) => None,
                    (Some(PlanGroup::Bound(group)), _) => Some(Planned::Made(group.clone())),
                    (Some(group), Some(pipeline)) => Some(make(&Owner::Render(pipeline.clone()), Frequency::Pass, group)?),
                    (Some(PlanGroup::Make(_)), None) => {
                        return Err("its work makes a pass group and names no pipeline to lay it out for".to_owned());
                    }
                };
                Ok(PlannedWork::Draw {
                    tags: tags.clone(),
                    pass,
                })
            }
            (Work::Fullscreen { pipeline, groups }, PassKind::Fullscreen) => {
                check_fullscreen(graph, pass, pipeline)?;
                Ok(PlannedWork::Fullscreen {
                    pipeline: pipeline.clone(),
                    groups: make_all(Owner::Render(pipeline.clone()), groups)?,
                })
            }
            (
                Work::Compute {
                    pipeline,
                    groups,
                    workgroups,
                },
                PassKind::Compute,
            ) => {
                self.check_workgroups(*workgroups).map_err(|problem| problem.to_string())?;
                Ok(PlannedWork::Compute {
                    pipeline: pipeline.clone(),
                    groups: make_all(Owner::Compute(pipeline.clone()), groups)?,
                    workgroups: *workgroups,
                })
            }
            (Work::Copy, PassKind::Copy) => {
                check_copy(graph, pass, viewport)?;
                Ok(PlannedWork::Copy)
            }
            (work, kind) => {
                let does = match work {
                    Work::Draw { .. } => "draws draw sets",
                    Work::Fullscreen { .. } => "is fullscreen",
                    Work::Compute { .. } => "dispatches",
                    Work::Copy => "copies",
                };
                Err(format!("it is a {kind} pass, and its work {does}"))
            }
        }
    }

    /// `group`, for `owner`'s layout of `frequency`, made now unless it binds an imported
    /// resource.
    fn planned(
        &self,
        graph: &RenderGraph,
        allocations: &[Option<Allocation>],
        pass: &Pass,
        owner: &Owner,
        frequency: Frequency,
        group: &PlanGroup,
    ) -> Result<Planned, String> {
        let bindings = match group {
            PlanGroup::Bound(group) => return Ok(Planned::Made(group.clone())),
            PlanGroup::Make(bindings) => bindings,
        };
        let mut imported = false;
        for binding in bindings {
            let (name, want) = match binding {
                PlanBinding::Target(_, name) => (name, Use::Sampled),
                PlanBinding::Buffer(id, name) => {
                    let access = owner.group(frequency).slots.iter().find_map(|(_, slot, _)| match slot {
                        Slot::Storage(declared, access, _) if declared == id => Some(*access),
                        _ => None,
                    });
                    // An undeclared buffer is the bind group's to refuse.
                    let Some(access) = access else {
                        continue;
                    };
                    (name, Use::Storage(access))
                }
                _ => continue,
            };
            let Some(resource) = graph.resource_id(name) else {
                return Err(format!(
                    "its {frequency} group binds {name}, which the graph has no target or buffer called"
                ));
            };
            check_use(pass, resource.index(), name, frequency, want)?;
            imported |= graph.resources()[resource.index()].origin == ResourceOrigin::Imported;
        }
        if imported {
            return Ok(Planned::EachRun {
                owner: owner.clone(),
                frequency,
                bindings: bindings.clone(),
            });
        }
        let live = |name: &str| {
            graph
                .resource_id(name)
                .and_then(|id| allocations[id.index()].as_ref().map(Allocation::live))
        };
        self.make_group(owner.group(frequency), bindings, &live)
            .map(|group| Planned::Made(Arc::new(group)))
            .map_err(|problem| problem.to_string())
    }

    /// The group `bindings` make for `group`'s layout, with `live` finding each graph resource.
    fn make_group<'a>(
        &self,
        group: &Group,
        bindings: &'a [PlanBinding],
        live: &dyn Fn(&str) -> Option<Live<'a>>,
    ) -> Result<BindGroup, GpuError> {
        let missing = |name: &str| GpuError::Invalid(format!("the run gives nothing for {name}"));
        let mut bound = Vec::with_capacity(bindings.len());
        for binding in bindings {
            bound.push(match binding {
                PlanBinding::Uniform(buffer) => Binding::Uniform(buffer),
                PlanBinding::Texture(id, texture) => Binding::Texture(*id, texture),
                PlanBinding::Sampler(id, sampler) => Binding::Sampler(*id, sampler),
                PlanBinding::Storage(id, buffer) => Binding::Storage(*id, buffer),
                PlanBinding::Target(id, name) => match live(name) {
                    Some(Live::Texture(texture)) => Binding::Texture(*id, texture),
                    Some(Live::Buffer(_)) => {
                        return Err(GpuError::Invalid(format!("{name} is a buffer, bound as a texture")));
                    }
                    None => return Err(missing(name)),
                },
                PlanBinding::Buffer(id, name) => match live(name) {
                    Some(Live::Buffer(buffer)) => Binding::Storage(*id, buffer),
                    Some(Live::Texture(_)) => {
                        return Err(GpuError::Invalid(format!("{name} is a target, bound as a buffer")));
                    }
                    None => return Err(missing(name)),
                },
            });
        }
        self.bind_group_of(group, &bound)
    }

    /// Runs `plan` once: every surviving pass, in order, in one submission, drawing the sets of
    /// `sets` its draw passes choose, into its own resources and `imports`.
    ///
    /// # Errors
    ///
    /// An imported resource used and not given, given twice, given as the wrong kind, or given a
    /// texture of another format or size or a buffer of another size, or one not made to be used
    /// as the plan uses it; a pass that draws a set whose pipeline's targets aren't the pass's,
    /// that writes depth the pass only tests against, or whose program binds a pass group the
    /// pass doesn't give.
    pub fn run(&self, plan: &Plan, sets: &DrawSets, imports: &[Import<'_>]) -> Result<(), GpuError> {
        let graph = &plan.graph;
        let live = plan.with_imports(imports)?;

        // Everything each pass records with, held while the passes are recorded.
        let mut held = Vec::with_capacity(plan.passes.len());
        for planned in &plan.passes {
            let pass = &graph.passes()[planned.index];
            let name = pass.name();
            let used = pass.inputs.iter().map(|input| input.resource);
            for resource in used.chain(pass.outputs.iter().map(|output| output.resource)) {
                if live[resource.index()].is_none() {
                    let missing = &graph.resources()[resource.index()].name;
                    return Err(in_pass(name, format!("the run gives nothing for {missing}")));
                }
            }
            let made = |group: &Planned| -> Result<Arc<BindGroup>, GpuError> {
                match group {
                    Planned::Made(group) => Ok(group.clone()),
                    Planned::EachRun {
                        owner,
                        frequency,
                        bindings,
                    } => {
                        let find = |target: &str| graph.resource_id(target).and_then(|id| live[id.index()]);
                        self.make_group(owner.group(*frequency), bindings, &find)
                            .map(Arc::new)
                            .map_err(|problem| in_pass(name, problem))
                    }
                }
            };
            let made_all = |groups: &[Option<Planned>; 3]| -> Result<[Option<Arc<BindGroup>>; 3], GpuError> {
                Ok([
                    groups[0].as_ref().map(made).transpose()?,
                    groups[1].as_ref().map(made).transpose()?,
                    groups[2].as_ref().map(made).transpose()?,
                ])
            };
            held.push(match &planned.work {
                PlannedWork::Draw { tags, pass: group } => Held::Draw {
                    sets: sets.matching(tags),
                    pass: group.as_ref().map(made).transpose()?,
                },
                PlannedWork::Fullscreen { pipeline, groups } => Held::Fullscreen {
                    pipeline: pipeline.clone(),
                    groups: made_all(groups)?,
                },
                PlannedWork::Compute {
                    pipeline,
                    groups,
                    workgroups,
                } => Held::Compute {
                    pipeline: pipeline.clone(),
                    groups: made_all(groups)?,
                    workgroups: *workgroups,
                },
                PlannedWork::Copy => Held::Copy,
            });
        }

        let mut recorded = Vec::with_capacity(plan.passes.len());
        for (planned, held) in plan.passes.iter().zip(&held) {
            let pass = &graph.passes()[planned.index];
            recorded.push(record(pass, held, &live).map_err(|problem| in_pass(pass.name(), problem))?);
        }
        self.checked(|| {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            for (position, (planned, recording)) in plan.passes.iter().zip(&recorded).enumerate() {
                let timed = plan
                    .timing
                    .as_ref()
                    .and_then(|timing| timing.first[position].map(|first| (&timing.set, first)));
                recording.encode(&mut encoder, &graph.passes()[planned.index], &live, timed);
            }
            if let Some(timing) = &plan.timing {
                encoder.resolve_query_set(&timing.set, 0..timing.count, &timing.resolve, 0);
            }
            self.queue.submit([encoder.finish()]);
        })
    }
}

/// What a group binding a graph resource needs its pass to do with it.
#[derive(Clone, Copy, Debug)]
enum Use {
    /// Read it as a texture.
    Sampled,
    /// Read it, for a read-only declaration, or write it, for a read-write one.
    Storage(Access),
}

/// Whether `pass` uses the resource at `index`, called `name`, as a group binding it `want`s.
fn check_use(pass: &Pass, index: usize, name: &str, frequency: Frequency, want: Use) -> Result<(), String> {
    let reads = |usages: &[ReadUsage]| {
        pass.inputs
            .iter()
            .any(|input| input.resource.index() == index && usages.contains(&input.usage))
    };
    let stores = pass
        .outputs
        .iter()
        .any(|output| output.resource.index() == index && output.attachment == Attachment::Storage);
    let (used, declare) = match want {
        Use::Sampled => (
            reads(&[ReadUsage::Texture, ReadUsage::DepthTexture]),
            "a texture or depth texture read",
        ),
        Use::Storage(Access::Read) => (reads(&[ReadUsage::Buffer]), "a buffer read"),
        Use::Storage(Access::ReadWrite) => (stores, "a storage write"),
    };
    if used {
        return Ok(());
    }
    Err(format!(
        "its {frequency} group binds {name}, which the pass doesn't declare as {declare}"
    ))
}

/// What one pass records with in one run.
enum Held {
    Draw {
        sets: Vec<Arc<DrawSet>>,
        pass: Option<Arc<BindGroup>>,
    },
    Fullscreen {
        pipeline: Arc<RenderPipeline>,
        groups: [Option<Arc<BindGroup>>; 3],
    },
    Compute {
        pipeline: Arc<ComputePipeline>,
        groups: [Option<Arc<BindGroup>>; 3],
        workgroups: [u32; 3],
    },
    Copy,
}

/// One pass of one run, checked and ready to encode.
enum Recording<'a> {
    Draw {
        colors: Vec<Target<'a>>,
        depth: Option<Target<'a>>,
        read_only: bool,
        draws: Vec<Draw<'a>>,
    },
    Compute {
        pipeline: &'a ComputePipeline,
        groups: [Option<&'a BindGroup>; 3],
        workgroups: [u32; 3],
    },
    Copy,
}

/// `pass` with what it holds, checked as a pass or a dispatch is.
fn record<'a>(pass: &Pass, held: &'a Held, live: &[Option<Live<'a>>]) -> Result<Recording<'a>, String> {
    let texture = |index: usize| match live[index] {
        Some(Live::Texture(texture)) => texture,
        _ => unreachable!("the graph's kinds and the run's checks leave a texture here"),
    };
    let (pipeline_groups, draws) = match held {
        Held::Draw { sets, pass } => (None, sets.iter().map(|set| set.draw(pass.as_deref())).collect()),
        Held::Fullscreen { pipeline, groups } => (
            None,
            vec![Draw {
                groups: [groups[0].as_deref(), groups[1].as_deref(), groups[2].as_deref()],
                ..Draw::new(pipeline, 3)
            }],
        ),
        Held::Compute {
            pipeline,
            groups,
            workgroups,
        } => (Some((pipeline, groups, workgroups)), Vec::new()),
        Held::Copy => return Ok(Recording::Copy),
    };
    if let Some((pipeline, groups, workgroups)) = pipeline_groups {
        let groups = [groups[0].as_deref(), groups[1].as_deref(), groups[2].as_deref()];
        if let Some(problem) = compute::group_problem(pipeline, groups) {
            return Err(format!("its dispatch is invalid: {problem}"));
        }
        return Ok(Recording::Compute {
            pipeline,
            groups,
            workgroups: *workgroups,
        });
    }

    let mut colors = Vec::new();
    let mut depth = None;
    for output in &pass.outputs {
        let target = Target {
            texture: texture(output.resource.index()),
            level: 0,
            layer: 0,
            load: output.load,
            store: output.store,
        };
        match output.attachment {
            Attachment::Color => colors.push(target),
            Attachment::Depth => depth = Some(target),
            Attachment::Storage | Attachment::Copy => unreachable!("a drawing pass writes only attachments"),
        }
    }
    let tested = pass.inputs.iter().find(|input| input.usage == ReadUsage::DepthTest);
    if let Some(input) = tested {
        depth = Some(Target {
            texture: texture(input.resource.index()),
            level: 0,
            layer: 0,
            load: LoadOp::Load,
            store: StoreOp::Store,
        });
    }
    check_pass(&colors, depth.as_ref(), tested.is_some(), &draws).map_err(|problem| problem.to_string())?;
    Ok(Recording::Draw {
        colors,
        depth,
        read_only: tested.is_some(),
        draws,
    })
}

impl Recording<'_> {
    /// Records the pass into `encoder`: the buffers it clears, then its work.
    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pass: &Pass,
        live: &[Option<Live<'_>>],
        timed: Option<(&wgpu::QuerySet, u32)>,
    ) {
        for output in &pass.outputs {
            if let (LoadOp::Clear(_), Some(Live::Buffer(buffer))) = (output.load, live[output.resource.index()]) {
                encoder.clear_buffer(&buffer.raw, 0, None);
            }
        }
        match self {
            Recording::Draw {
                colors,
                depth,
                read_only,
                draws,
            } => encode(encoder, colors, depth.as_ref(), *read_only, draws, timed),
            Recording::Compute {
                pipeline,
                groups,
                workgroups,
            } => compute::encode(encoder, pipeline, *groups, *workgroups, timed),
            Recording::Copy => {
                let (source, destination) = (pass.inputs[0].resource.index(), pass.outputs[0].resource.index());
                let (Some(from), Some(to)) = (live[source], live[destination]) else {
                    unreachable!("the run checked every resource a pass uses is there");
                };
                encode_copy(encoder, from, to);
            }
        }
    }
}

/// Records a copy of `from` into `to`, rows padded as [`copy_row`] says between a target and a
/// buffer.
fn encode_copy(encoder: &mut wgpu::CommandEncoder, from: Live<'_>, to: Live<'_>) {
    fn image(texture: &Texture) -> wgpu::TexelCopyTextureInfo<'_> {
        wgpu::TexelCopyTextureInfo {
            texture: &texture.raw,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: if texture.desc().format.is_depth() {
                wgpu::TextureAspect::DepthOnly
            } else {
                wgpu::TextureAspect::All
            },
        }
    }
    let layout = |texture: &Texture| wgpu::TexelCopyBufferLayout {
        offset: 0,
        bytes_per_row: copy_row(texture.desc().format, texture.desc().width),
        rows_per_image: Some(texture.desc().height),
    };
    let extent = |texture: &Texture| wgpu::Extent3d {
        width: texture.desc().width,
        height: texture.desc().height,
        depth_or_array_layers: 1,
    };
    match (from, to) {
        (Live::Buffer(from), Live::Buffer(to)) => encoder.copy_buffer_to_buffer(&from.raw, 0, &to.raw, 0, from.size()),
        (Live::Texture(from), Live::Texture(to)) => encoder.copy_texture_to_texture(image(from), image(to), extent(from)),
        (Live::Texture(from), Live::Buffer(to)) => encoder.copy_texture_to_buffer(
            image(from),
            wgpu::TexelCopyBufferInfo {
                buffer: &to.raw,
                layout: layout(from),
            },
            extent(from),
        ),
        (Live::Buffer(from), Live::Texture(to)) => encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &from.raw,
                layout: layout(to),
            },
            image(to),
            extent(to),
        ),
    }
}

/// Whether a fullscreen pipeline draws into its pass's targets.
fn check_fullscreen(graph: &RenderGraph, pass: &Pass, pipeline: &RenderPipeline) -> Result<(), String> {
    let format = |index: usize| graph.resources()[index].format();
    let colors: Vec<_> = pass
        .outputs
        .iter()
        .filter(|output| output.attachment == Attachment::Color)
        .filter_map(|output| format(output.resource.index()))
        .collect();
    let written = pass
        .outputs
        .iter()
        .filter(|output| output.attachment == Attachment::Depth)
        .map(|output| output.resource);
    let tested = pass
        .inputs
        .iter()
        .filter(|input| input.usage == ReadUsage::DepthTest)
        .map(|input| input.resource);
    let depth = written.chain(tested).find_map(|resource| format(resource.index()));
    let state = pipeline.state();
    let wanted: Vec<_> = state.colors.iter().map(|color| color.format).collect();
    let wanted_depth = state.depth.map(|depth| depth.format);
    if wanted != colors || wanted_depth != depth {
        return Err(format!(
            "its pipeline draws into {wanted:?} and depth {wanted_depth:?}, and the pass into {colors:?} and depth {depth:?}"
        ));
    }
    Ok(())
}

/// Whether a copy between a target and a buffer fits at `viewport`, and doesn't write a depth
/// target from a buffer, which no backend can.
fn check_copy(graph: &RenderGraph, pass: &Pass, viewport: Extent) -> Result<(), String> {
    let source = &graph.resources()[pass.inputs[0].resource.index()];
    let destination = &graph.resources()[pass.outputs[0].resource.index()];
    let (target, buffer) = match (source.desc, destination.desc) {
        (ResourceDesc::Texture { .. }, ResourceDesc::Buffer { .. }) => (source, destination),
        (ResourceDesc::Buffer { .. }, ResourceDesc::Texture { format, .. }) => {
            if format.is_depth() {
                return Err(format!(
                    "it copies a buffer into {}, a depth target, which only a pass can write",
                    destination.name
                ));
            }
            (destination, source)
        }
        _ => return Ok(()),
    };
    let (Some(format), Some(size), Some(bytes)) = (target.format(), target.size(), buffer.bytes()) else {
        unreachable!("one is a target and the other a buffer");
    };
    let size = size.resolve(viewport);
    let Some(row) = copy_row(format, size.width) else {
        unreachable!("the graph refuses copies of formats with no layout in a buffer");
    };
    let needed = u64::from(row) * u64::from(size.height);
    if bytes < needed {
        return Err(format!(
            "{} at {}×{} takes {needed} bytes in a buffer, rows of {row}, and {} holds {bytes}",
            target.name, size.width, size.height, buffer.name
        ));
    }
    Ok(())
}
