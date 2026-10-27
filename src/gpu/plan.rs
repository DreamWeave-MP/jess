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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::bind::{BindGroup, Binding};
use super::compute::{self, ComputePipeline};
use super::draw_set::{DrawSet, DrawSets};
use super::pass::{check_pass, encode};
use super::pipeline::Slot;
use super::texture::{TextureDesc, TextureShape, TextureUsage};
use super::{Buffer, BufferUsage, Draw, Gpu, GpuError, Group, RenderPipeline, Resolve, Sampler, Target, Texture};
use crate::render::{
    AliasGroup, Attachment, CopyRegion, Extent, LoadOp, Pass, PassKind, ReadUsage, RenderGraph, ResolvedCopy, ResourceDesc,
    ResourceOrigin, StoreOp, Subresource, TargetDesc, TexelAt,
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
    /// A declared texture, bound to the graph's target of this name, which the pass reads: all of
    /// it, or the one level of one layer the pass reads.
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
pub enum Live<'a> {
    Texture(&'a Texture),
    Buffer(&'a Buffer),
}

impl Live<'_> {
    /// The identity of the texture or buffer.
    fn id(self) -> u64 {
        match self {
            Live::Texture(texture) => texture.id(),
            Live::Buffer(buffer) => buffer.id(),
        }
    }
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
    /// Made for a run, because it binds an imported resource: made again only when a run gives
    /// other resources than the last.
    EachRun {
        owner: Owner,
        frequency: Frequency,
        bindings: Vec<PlanBinding>,
        /// The group last made, and the identities of the graph resources it bound, in order.
        made: Mutex<Option<(Vec<u64>, Arc<BindGroup>)>>,
    },
}

/// The sets a draw pass last chose: from which collection, at which version, shown or hidden.
type Matched = Option<(u64, u64, Arc<[Arc<DrawSet>]>)>;

/// What a surviving pass does, as it was planned.
#[derive(Debug)]
enum PlannedWork {
    Draw {
        tags: Vec<String>,
        pass: Option<Planned>,
        matched: Mutex<Matched>,
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
    /// What its region resolved to at the plan's viewport, and the version of the plan's
    /// controls it was resolved for.
    Copy(Mutex<(u64, ResolvedCopy)>),
}

/// What a script changes about a plan between runs, shared by the plan and every plan resized
/// from it, so a window showing it at its own size follows along.
#[derive(Debug)]
struct Controls {
    /// Whether runs record each surviving pass, in order.
    enabled: Vec<AtomicBool>,
    /// The region each copy pass copies, where it isn't the graph's, by where it runs.
    regions: Mutex<Vec<Option<CopyRegion>>>,
    /// Counts every change of a region.
    version: AtomicU64,
}

/// A pass that survived, ready to record.
#[derive(Debug)]
struct PlannedPass {
    /// Its index in the graph.
    index: usize,
    work: PlannedWork,
    /// Where the passes whose contents it reads or loads run, in the plan's order.
    sources: Vec<usize>,
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
    /// Which passes the last run recorded, in order.
    ran: Mutex<Vec<bool>>,
    controls: Arc<Controls>,
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
    /// timed, and for a pass the run didn't record.
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
        let plan = gpu.plan_with(&self.graph, self.work.clone(), viewport, Some(Arc::clone(&self.controls)))?;
        for (position, planned) in plan.passes.iter().enumerate() {
            if let PlannedWork::Copy(copy) = &planned.work {
                plan.copy(position, copy)?;
            }
        }
        Ok(plan)
    }

    /// What the copy pass at `position`, whose resolved copy is `copy`, copies now: its region,
    /// resolved again if it changed.
    fn copy(&self, position: usize, copy: &Mutex<(u64, ResolvedCopy)>) -> Result<ResolvedCopy, GpuError> {
        let version = self.controls.version.load(Ordering::Acquire);
        let (resolved_for, resolved) = *lock(copy);
        if resolved_for == version {
            return Ok(resolved);
        }
        let pass = &self.graph.passes()[self.passes[position].index];
        let region = lock(&self.controls.regions)[position].unwrap_or(pass.region);
        let resolved = self
            .graph
            .resolve_copy(pass, region, Some(self.viewport))
            .map_err(|problem| in_pass(pass.name(), problem))?
            .expect("a viewport resolves every copy");
        *lock(copy) = (version, resolved);
        Ok(resolved)
    }

    /// The resource called `name`.
    fn resource(&self, name: &str) -> Result<(usize, &crate::render::Resource), GpuError> {
        let id = self
            .graph
            .resource_id(name)
            .ok_or_else(|| GpuError::Invalid(format!("the plan has no target or buffer called {name}")))?;
        Ok((id.index(), &self.graph.resources()[id.index()]))
    }

    /// How many bytes the buffer `name` holds at the plan's viewport.
    ///
    /// # Errors
    ///
    /// A name the plan has no buffer by.
    pub fn buffer_bytes(&self, name: &str) -> Result<u64, GpuError> {
        let (_, resource) = self.resource(name)?;
        resource
            .bytes_at(Some(self.viewport))
            .ok_or_else(|| GpuError::Invalid(format!("{name} is a target, not a buffer")))
    }

    /// How big level 0 of the target `name` is at the plan's viewport.
    ///
    /// # Errors
    ///
    /// A name the plan has no target by.
    pub fn target_extent(&self, name: &str) -> Result<Extent, GpuError> {
        let (_, resource) = self.resource(name)?;
        resource
            .target()
            .map(|target| target.resolve(self.viewport))
            .ok_or_else(|| GpuError::Invalid(format!("{name} is a buffer, not a target")))
    }

    /// The plan's own texture or buffer for the kept resource `name`.
    pub(crate) fn kept(&self, name: &str) -> Result<Live<'_>, GpuError> {
        let (index, resource) = self.resource(name)?;
        if resource.origin != ResourceOrigin::Kept {
            return Err(GpuError::Invalid(format!(
                "{name} is {}, not kept: only the plan's kept resources are read from it",
                resource.origin.name()
            )));
        }
        self.allocations[index]
            .as_ref()
            .map(Allocation::live)
            .ok_or_else(|| GpuError::Invalid(format!("{name} was never made")))
    }

    /// Where the surviving pass `name` runs.
    fn position(&self, name: &str) -> Result<usize, GpuError> {
        let Some(id) = self.graph.pass_id(name) else {
            return Err(GpuError::Invalid(format!("the plan has no pass called {name}")));
        };
        self.passes
            .iter()
            .position(|planned| planned.index == id.index())
            .ok_or_else(|| GpuError::Invalid(format!("pass {name} is culled: nothing sees what it does, so it never runs")))
    }

    /// Whether runs record the surviving pass `name`.
    ///
    /// # Errors
    ///
    /// A pass the plan doesn't have, or culled.
    pub fn enabled(&self, name: &str) -> Result<bool, GpuError> {
        Ok(self.controls.enabled[self.position(name)?].load(Ordering::Relaxed))
    }

    /// Records `names` in later runs, or leaves them out, together. Every pass is recorded until
    /// this says otherwise. A pass left out does nothing: whatever it would have stored keeps
    /// what it held, which a pass still recorded mustn't depend on.
    ///
    /// # Errors
    ///
    /// A pass the plan doesn't have, or culled; or a change after which a recorded pass would
    /// read or load what a pass left out stores, which changes nothing.
    pub fn set_enabled(&self, names: &[&str], enabled: bool) -> Result<(), GpuError> {
        let mut state: Vec<bool> = self
            .controls
            .enabled
            .iter()
            .map(|enabled| enabled.load(Ordering::Relaxed))
            .collect();
        for name in names {
            state[self.position(name)?] = enabled;
        }
        for (position, planned) in self.passes.iter().enumerate() {
            let source = planned.sources.iter().find(|&&source| !state[source]);
            if let (true, Some(&source)) = (state[position], source) {
                let name = |at: usize| self.graph.passes()[self.passes[at].index].name();
                return Err(GpuError::Invalid(format!(
                    "pass {} uses what pass {} stores: leave both out, or neither",
                    name(position),
                    name(source)
                )));
            }
        }
        for (control, enabled) in self.controls.enabled.iter().zip(state) {
            control.store(enabled, Ordering::Relaxed);
        }
        Ok(())
    }

    /// Copies `region` in the copy pass `name` from the next run on, in this plan and every plan
    /// resized from it or it from: a part of the same level of the same layer, checked now at the
    /// plan's viewport, and resolved again at each plan's own.
    ///
    /// # Errors
    ///
    /// A pass the plan doesn't have, culled, or not a copy; a region that doesn't fit, as
    /// [`RenderGraph::resolve_copy`] says, which changes nothing.
    pub fn set_region(&self, name: &str, region: CopyRegion) -> Result<(), GpuError> {
        let position = self.position(name)?;
        let planned = &self.passes[position];
        if !matches!(planned.work, PlannedWork::Copy(_)) {
            return Err(GpuError::Invalid(format!("pass {name} isn't a copy, so it has no region")));
        }
        let pass = &self.graph.passes()[planned.index];
        self.graph
            .resolve_copy(pass, region, Some(self.viewport))
            .map_err(|problem| in_pass(name, problem))?;
        lock(&self.controls.regions)[position] = Some(region);
        self.controls.version.fetch_add(1, Ordering::AcqRel);
        Ok(())
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
                (Import::Texture(_, texture), ResourceDesc::Texture(target)) => {
                    let desc = texture.desc();
                    let size = target.resolve(self.viewport);
                    let given = Extent {
                        width: desc.width,
                        height: desc.height,
                    };
                    if desc.format != target.format || given != size {
                        return Err(GpuError::Invalid(format!(
                            "the run gives {name} a {}×{} {} texture: expected {}×{} {}",
                            given.width, given.height, desc.format, size.width, size.height, target.format
                        )));
                    }
                    let shape = (desc.mips, desc.shape.layers(), desc.samples);
                    let wanted = (target.mips, target.layers, target.samples);
                    if shape != wanted || matches!(desc.shape, TextureShape::D3 { .. }) {
                        return Err(GpuError::Invalid(format!(
                            "the run gives {name} a texture of {} levels, {} layers, and {} samples: expected {}, {}, and {}",
                            shape.0, shape.1, shape.2, wanted.0, wanted.1, wanted.2
                        )));
                    }
                    Live::Texture(texture)
                }
                (Import::Buffer(_, buffer), ResourceDesc::Buffer(size)) => {
                    let bytes = size.resolve(self.viewport);
                    if buffer.size() != bytes {
                        return Err(GpuError::Invalid(format!(
                            "the run gives {name} a {}-byte buffer: expected {bytes} bytes",
                            buffer.size()
                        )));
                    }
                    Live::Buffer(buffer)
                }
                (Import::Texture(..), ResourceDesc::Buffer(_)) => {
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

impl Gpu {
    /// Compiles `graph` with `work`, one for each of its passes in the order they were added,
    /// for a viewport of `viewport` pixels, or as many more as its relative targets' mip levels
    /// need.
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
        self.plan_with(graph, work, viewport, None)
    }

    /// A plan of `graph` with `controls`, a plan's it was resized from, or new ones.
    fn plan_with(
        &self,
        graph: &RenderGraph,
        work: Vec<Work>,
        viewport: Extent,
        controls: Option<Arc<Controls>>,
    ) -> Result<Plan, GpuError> {
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
            let mut sources: Vec<usize> = compiled
                .dependencies()
                .iter()
                .filter(|dependency| dependency.after == *id && dependency.kind.carries_contents())
                .filter_map(|dependency| compiled.position(dependency.before))
                .collect();
            sources.dedup();
            passes.push(PlannedPass {
                index,
                work: planned,
                sources,
            });
        }
        let timing = self.timing(&passes)?;
        let controls = controls.unwrap_or_else(|| {
            Arc::new(Controls {
                enabled: passes.iter().map(|_| AtomicBool::new(true)).collect(),
                regions: Mutex::new(vec![None; passes.len()]),
                version: AtomicU64::new(0),
            })
        });
        Ok(Plan {
            controls,
            graph: graph.clone(),
            work,
            viewport,
            allocations,
            ran: Mutex::new(vec![false; passes.len()]),
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
                if matches!(pass.work, PlannedWork::Copy(_)) {
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
        let ran = lock(&plan.ran).clone();
        Ok(Some(
            plan.passes
                .iter()
                .zip(&timing.first)
                .zip(ran)
                .map(|((pass, first), ran)| PassTiming {
                    pass: plan.graph.passes()[pass.index].name().to_owned(),
                    // Ticks between two stamps of one pass are far below 2^53.
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    duration: first.filter(|_| ran).map(|first| {
                        let ticks = stamp(first + 1).saturating_sub(stamp(first));
                        std::time::Duration::from_nanos((ticks as f64 * period) as u64)
                    }),
                })
                .collect(),
        ))
    }

    /// A texture or buffer for each group of transient resources that share one, and for each kept
    /// resource, at its size for `viewport`.
    fn allocate(
        &self,
        graph: &RenderGraph,
        groups: &[AliasGroup],
        viewport: Extent,
    ) -> Result<Vec<Option<Allocation>>, GpuError> {
        let resources = graph.resources();
        let mut allocations = vec![None; resources.len()];
        let shared = groups
            .iter()
            .map(|group| group.resources.iter().map(|member| member.index()).collect::<Vec<_>>());
        let kept = (0..resources.len())
            .filter(|&index| resources[index].origin == ResourceOrigin::Kept)
            .map(|index| vec![index]);
        for members in shared.chain(kept) {
            let first = &resources[members[0]];
            let allocation = match first.desc {
                ResourceDesc::Texture(target) => self
                    .texture(&texture_desc(target, viewport))
                    .map(|texture| Allocation::Texture(Arc::new(texture))),
                ResourceDesc::Buffer(size) => {
                    let usage = BufferUsage::VERTEX
                        | BufferUsage::INDEX
                        | BufferUsage::UNIFORM
                        | BufferUsage::STORAGE
                        | BufferUsage::INDIRECT;
                    self.buffer(size.resolve(viewport), usage)
                        .map(|buffer| Allocation::Buffer(Arc::new(buffer)))
                }
            }
            .map_err(|problem| GpuError::Invalid(format!("{} can't be made: {problem}", first.name)))?;
            for member in members {
                allocations[member] = Some(allocation.clone());
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
                    matched: Mutex::new(None),
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
                let copy = graph.resolve_copy(pass, pass.region, Some(viewport))?;
                let copy = copy.expect("a viewport resolves every copy");
                Ok(PlannedWork::Copy(Mutex::new((0, copy))))
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
                made: Mutex::new(None),
            });
        }
        let live = |name: &str| {
            graph
                .resource_id(name)
                .and_then(|id| allocations[id.index()].as_ref().map(Allocation::live))
        };
        self.make_group(owner.group(frequency), bindings, &live, &|name| read_level(graph, pass, name))
            .map(|group| Planned::Made(Arc::new(group)))
            .map_err(|problem| problem.to_string())
    }

    /// The group `bindings` make for `group`'s layout, with `live` finding each graph resource
    /// and `level` the level of a layer of a target the pass reads only one of.
    fn make_group<'a>(
        &self,
        group: &Group,
        bindings: &'a [PlanBinding],
        live: &dyn Fn(&str) -> Option<Live<'a>>,
        level: &dyn Fn(&str) -> Option<Subresource>,
    ) -> Result<BindGroup, GpuError> {
        let missing = |name: &str| GpuError::Invalid(format!("the run gives nothing for {name}"));
        let mut bound = Vec::with_capacity(bindings.len());
        for binding in bindings {
            bound.push(match binding {
                PlanBinding::Uniform(buffer) => Binding::Uniform(buffer),
                PlanBinding::Texture(id, texture) => Binding::Texture(*id, texture),
                PlanBinding::Sampler(id, sampler) => Binding::Sampler(*id, sampler),
                PlanBinding::Storage(id, buffer) => Binding::Storage(*id, buffer),
                PlanBinding::Target(id, name) => match (live(name), level(name)) {
                    (Some(Live::Texture(texture)), None) => Binding::Texture(*id, texture),
                    (Some(Live::Texture(texture)), Some(one)) => Binding::Level(*id, texture, one.level, one.layer),
                    (Some(Live::Buffer(_)), _) => {
                        return Err(GpuError::Invalid(format!("{name} is a buffer, bound as a texture")));
                    }
                    (None, _) => return Err(missing(name)),
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

        // The passes it records, by where they run, and everything each records with, held while
        // the passes are recorded.
        let running: Vec<usize> = (0..plan.passes.len())
            .filter(|&position| plan.controls.enabled[position].load(Ordering::Relaxed))
            .collect();
        let mut held = Vec::with_capacity(running.len());
        for &position in &running {
            held.push(self.hold(plan, sets, &live, position)?);
        }

        let mut recorded = Vec::with_capacity(running.len());
        for (&position, held) in running.iter().zip(&held) {
            let pass = &graph.passes()[plan.passes[position].index];
            recorded.push(record(pass, held, &live).map_err(|problem| in_pass(pass.name(), problem))?);
        }
        {
            let mut ran = lock(&plan.ran);
            ran.fill(false);
            for &position in &running {
                ran[position] = true;
            }
        }
        self.checked(|| {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            for (&position, recording) in running.iter().zip(&recorded) {
                let planned = &plan.passes[position];
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

    /// What the pass at `position` of `plan` records with in a run of `live` drawing from `sets`.
    fn hold(&self, plan: &Plan, sets: &DrawSets, live: &[Option<Live<'_>>], position: usize) -> Result<Held, GpuError> {
        let graph = &plan.graph;
        let planned = &plan.passes[position];
        let pass = &graph.passes()[planned.index];
        let name = pass.name();
        for resource in pass.resources() {
            if live[resource.index()].is_none() {
                let missing = &graph.resources()[resource.index()].name;
                return Err(in_pass(name, format!("the run gives nothing for {missing}")));
            }
        }
        let made = |group: &Planned| self.group_for(graph, pass, live, group);
        let made_all = |groups: &[Option<Planned>; 3]| -> Result<[Option<Arc<BindGroup>>; 3], GpuError> {
            Ok([
                groups[0].as_ref().map(made).transpose()?,
                groups[1].as_ref().map(made).transpose()?,
                groups[2].as_ref().map(made).transpose()?,
            ])
        };
        Ok(match &planned.work {
            PlannedWork::Draw {
                tags,
                pass: group,
                matched,
            } => Held::Draw {
                sets: chosen(matched, sets, tags),
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
            PlannedWork::Copy(copy) => Held::Copy(plan.copy(position, copy)?),
        })
    }

    /// The bind group `group` is for `pass` in a run of `live`: the one made already, or, for one
    /// binding an import, the one the last run made if it gave the same resources.
    fn group_for(
        &self,
        graph: &RenderGraph,
        pass: &Pass,
        live: &[Option<Live<'_>>],
        group: &Planned,
    ) -> Result<Arc<BindGroup>, GpuError> {
        let (owner, frequency, bindings, made) = match group {
            Planned::Made(group) => return Ok(group.clone()),
            Planned::EachRun {
                owner,
                frequency,
                bindings,
                made,
            } => (owner, frequency, bindings, made),
        };
        let find = |target: &str| graph.resource_id(target).and_then(|id| live[id.index()]);
        let bound = || {
            bindings.iter().filter_map(|binding| match binding {
                PlanBinding::Target(_, name) | PlanBinding::Buffer(_, name) => find(name).map(Live::id),
                _ => None,
            })
        };
        if let Some((identities, group)) = &*lock(made)
            && identities.iter().copied().eq(bound())
        {
            return Ok(group.clone());
        }
        let level = |target: &str| read_level(graph, pass, target);
        let group = self
            .make_group(owner.group(*frequency), bindings, &find, &level)
            .map(Arc::new)
            .map_err(|problem| in_pass(pass.name(), problem))?;
        *lock(made) = Some((bound().collect(), group.clone()));
        Ok(group)
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
        Use::Storage(Access::ReadWrite | Access::Atomic) => (stores, "a storage write"),
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
        sets: Arc<[Arc<DrawSet>]>,
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
    Copy(ResolvedCopy),
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
    Copy(ResolvedCopy),
}

/// `pass` with what it holds, checked as a pass or a dispatch is.
fn record<'a>(pass: &Pass, held: &'a Held, live: &[Option<Live<'a>>]) -> Result<Recording<'a>, String> {
    let texture = |index: usize| match live[index] {
        Some(Live::Texture(texture)) => texture,
        _ => unreachable!("the graph's kinds and the run's checks leave a texture here"),
    };
    let (pipeline_groups, draws) = match held {
        Held::Draw { sets, pass } => (
            None,
            sets.iter()
                .filter(|set| set.visible())
                .map(|set| set.draw(pass.as_deref()))
                .collect(),
        ),
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
        Held::Copy(copy) => return Ok(Recording::Copy(*copy)),
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
            level: output.subresource.level,
            layer: output.subresource.layer,
            load: output.load,
            store: output.store,
            resolve: output.resolve.map(|(resource, into)| Resolve {
                texture: texture(resource.index()),
                level: into.level,
                layer: into.layer,
            }),
        };
        match output.attachment {
            Attachment::Color => colors.push(target),
            Attachment::Depth => depth = Some(target),
            Attachment::Storage | Attachment::Copy => unreachable!("a drawing pass writes only attachments"),
        }
    }
    let tested = pass.inputs.iter().find(|input| input.usage == ReadUsage::DepthTest);
    if let Some(input) = tested {
        let one = input.one().unwrap_or_default();
        depth = Some(Target {
            texture: texture(input.resource.index()),
            level: one.level,
            layer: one.layer,
            load: LoadOp::Load,
            store: StoreOp::Store,
            resolve: None,
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
            Recording::Copy(copy) => {
                let (source, destination) = (pass.inputs[0].resource.index(), pass.outputs[0].resource.index());
                let (Some(from), Some(to)) = (live[source], live[destination]) else {
                    unreachable!("the run checked every resource a pass uses is there");
                };
                encode_copy(encoder, *copy, from, to);
            }
        }
    }
}

/// `mutex`'s value: a plan's state stays whole whatever panicked holding it.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The sets carrying any of `tags` in `sets`, chosen again only when `sets` is another
/// collection or has changed since the choice `matched` holds.
fn chosen(matched: &Mutex<Matched>, sets: &DrawSets, tags: &[String]) -> Arc<[Arc<DrawSet>]> {
    let mut matched = lock(matched);
    if let Some((id, version, chosen)) = &*matched
        && *id == sets.id()
        && *version == sets.version()
    {
        return chosen.clone();
    }
    // The version first: a change racing the choice makes the next run choose again.
    let version = sets.version();
    let chosen: Arc<[Arc<DrawSet>]> = sets.tagged(tags).into();
    *matched = Some((sets.id(), version, chosen.clone()));
    chosen
}

/// Records `copy` of `from` into `to`.
fn encode_copy(encoder: &mut wgpu::CommandEncoder, copy: ResolvedCopy, from: Live<'_>, to: Live<'_>) {
    fn image(texture: &Texture, at: TexelAt) -> wgpu::TexelCopyTextureInfo<'_> {
        wgpu::TexelCopyTextureInfo {
            texture: &texture.raw,
            mip_level: at.subresource.level,
            origin: wgpu::Origin3d {
                x: at.x,
                y: at.y,
                z: at.subresource.layer,
            },
            aspect: if texture.desc().format.is_depth() {
                wgpu::TextureAspect::DepthOnly
            } else {
                wgpu::TextureAspect::All
            },
        }
    }
    fn rows(buffer: &Buffer, offset: u64, row: u32, extent: Extent) -> wgpu::TexelCopyBufferInfo<'_> {
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer.raw,
            layout: wgpu::TexelCopyBufferLayout {
                offset,
                bytes_per_row: Some(row),
                rows_per_image: Some(extent.height),
            },
        }
    }
    let size = |extent: Extent| wgpu::Extent3d {
        width: extent.width,
        height: extent.height,
        depth_or_array_layers: 1,
    };
    match (copy, from, to) {
        (
            ResolvedCopy::Buffers {
                from: start,
                to: end,
                bytes,
            },
            Live::Buffer(from),
            Live::Buffer(to),
        ) => {
            encoder.copy_buffer_to_buffer(&from.raw, start, &to.raw, end, bytes);
        }
        (
            ResolvedCopy::Targets {
                from: start,
                to: end,
                extent,
            },
            Live::Texture(from),
            Live::Texture(to),
        ) => {
            encoder.copy_texture_to_texture(image(from, start), image(to, end), size(extent));
        }
        (
            ResolvedCopy::TargetToBuffer {
                from: start,
                offset,
                row,
                extent,
            },
            Live::Texture(from),
            Live::Buffer(to),
        ) => encoder.copy_texture_to_buffer(image(from, start), rows(to, offset, row, extent), size(extent)),
        (
            ResolvedCopy::BufferToTarget {
                offset,
                row,
                to: end,
                extent,
            },
            Live::Buffer(from),
            Live::Texture(to),
        ) => encoder.copy_buffer_to_texture(rows(from, offset, row, extent), image(to, end), size(extent)),
        _ => unreachable!("a copy resolves to the kinds of its resources"),
    }
}

/// The level of the layer of `name` that `pass` reads, when it reads one, not all of it.
fn read_level(graph: &RenderGraph, pass: &Pass, name: &str) -> Option<Subresource> {
    let resource = graph.resource_id(name)?;
    pass.inputs
        .iter()
        .find(|input| input.resource == resource && matches!(input.usage, ReadUsage::Texture | ReadUsage::DepthTexture))
        .and_then(|input| input.subresource)
}

/// The texture a transient target of `target` is at `viewport`: an array where it has several
/// layers, and sampled unless it is multisampled.
fn texture_desc(target: TargetDesc, viewport: Extent) -> TextureDesc {
    let size = target.resolve(viewport);
    let usage = if target.samples > 1 {
        TextureUsage::TARGET
    } else {
        TextureUsage::TARGET | TextureUsage::SAMPLED
    };
    TextureDesc {
        shape: if target.layers > 1 {
            TextureShape::D2Array { layers: target.layers }
        } else {
            TextureShape::D2
        },
        mips: target.mips,
        samples: target.samples,
        ..TextureDesc::d2(size.width, size.height, target.format, usage)
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
    // The graph has every attachment of a pass agree on its samples, so the first speaks for all.
    let attachment = pass.outputs.first().map(|output| output.resource);
    let samples = attachment.map_or(1, |resource| graph.resources()[resource.index()].samples());
    if samples != state.samples() {
        return Err(format!(
            "its pipeline draws {} samples a texel, and the pass's targets have {samples}",
            state.samples()
        ));
    }
    Ok(())
}
