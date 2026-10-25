//! Whole-graph checks, ordering, culling, lifetimes, and aliasing.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::render::error::GraphError;
use crate::render::graph::{PassId, RenderGraph, Resource, ResourceId, ResourceOrigin};
use crate::render::pass::{LoadOp, StoreOp};

/// Why one pass must run after another.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum DependencyKind {
    /// `after` reads the target, and `before` is its last writer.
    ReadAfterWrite,
    /// `after` writes the target with [`LoadOp::Load`], continuing what `before` wrote.
    LoadAfterWrite,
    /// `after` writes the target after `before` in registration order and replaces its contents.
    /// This orders the two; it passes nothing along.
    WriteAfterWrite,
}

impl DependencyKind {
    /// Whether `after` uses what `before` wrote, which keeps `before` alive through culling.
    #[must_use]
    pub const fn carries_contents(self) -> bool {
        !matches!(self, Self::WriteAfterWrite)
    }
}

/// One ordering edge between two passes, through one target.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Dependency {
    /// The pass that runs first.
    pub before: PassId,
    /// The pass that waits for it.
    pub after: PassId,
    /// The target that links them.
    pub resource: ResourceId,
    /// Why `after` waits.
    pub kind: DependencyKind,
}

/// The span of the compiled order in which a target is in use, as positions in
/// [`CompiledGraph::order`], both inclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Lifetime {
    /// Position of the first pass that uses the target.
    pub first: usize,
    /// Position of the last pass that uses the target.
    pub last: usize,
}

impl Lifetime {
    /// Whether the two spans share a pass. A target last used by a pass and one first used by
    /// the same pass overlap: both are bound at once.
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.first <= other.last && other.first <= self.last
    }
}

/// Transient resources that can share one allocation: targets of the same format and size, or
/// buffers of the same size, with pairwise disjoint lifetimes.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct AliasGroup {
    /// The members, in order of first use.
    pub resources: Vec<ResourceId>,
}

/// A checked graph: pass order, culled passes, and target lifetimes and aliasing.
///
/// `Display` is the debug dump: the graph's name, each pass in order with what it reads and
/// writes, the culled passes, every target's lifetime, and the alias groups.
#[derive(Clone, Debug)]
pub struct CompiledGraph<'graph> {
    pub(crate) graph: &'graph RenderGraph,
    pub(crate) order: Vec<PassId>,
    pub(crate) culled: Vec<PassId>,
    pub(crate) positions: Vec<Option<usize>>,
    pub(crate) dependencies: Vec<Dependency>,
    pub(crate) lifetimes: Vec<Option<Lifetime>>,
    pub(crate) alias_groups: Vec<AliasGroup>,
}

impl<'graph> CompiledGraph<'graph> {
    /// The graph this was compiled from.
    #[must_use]
    pub const fn graph(&self) -> &'graph RenderGraph {
        self.graph
    }

    /// The passes that survive culling, in execution order.
    #[must_use]
    pub fn order(&self) -> &[PassId] {
        &self.order
    }

    /// The passes nothing observes, in registration order.
    #[must_use]
    pub fn culled(&self) -> &[PassId] {
        &self.culled
    }

    /// Where `pass` runs in [`Self::order`], or `None` when it was culled or belongs to another
    /// graph.
    #[must_use]
    pub fn position(&self, pass: PassId) -> Option<usize> {
        self.positions.get(pass.0).copied().flatten()
    }

    /// The edges between surviving passes, sorted by where `after` runs, then where `before`
    /// runs, then target.
    #[must_use]
    pub fn dependencies(&self) -> &[Dependency] {
        &self.dependencies
    }

    /// When `resource` is in use, or `None` when only culled passes use it, no pass does, or it
    /// belongs to another graph.
    #[must_use]
    pub fn lifetime(&self, resource: ResourceId) -> Option<Lifetime> {
        self.lifetimes.get(resource.0).copied().flatten()
    }

    /// Transient targets grouped by the allocation they can share, in order of each group's first
    /// use. Every used transient target is in exactly one group; imported targets are in none.
    #[must_use]
    pub fn alias_groups(&self) -> &[AliasGroup] {
        &self.alias_groups
    }
}

/// Every pass's use of every target, in registration order.
struct Uses {
    /// Per target, `(pass, output)` for each write.
    writers: Vec<Vec<(usize, usize)>>,
    /// Per target, each pass that reads it.
    readers: Vec<Vec<usize>>,
}

impl RenderGraph {
    /// Checks the graph as a whole and orders it. The dependency rules are in the render module docs.
    ///
    /// # Errors
    ///
    /// When a transient target is unused, read without a writer, or loaded before it is written;
    /// when anything reads or loads contents a writer discarded; when the dependencies form a
    /// cycle; or when nothing reaches an imported target.
    pub fn compile(&self) -> Result<CompiledGraph<'_>, GraphError> {
        let uses = self.uses();
        self.check_contents(&uses)?;
        let dependencies = self.dependencies(&uses);
        let schedule = self.schedule(&dependencies)?;
        let live = self.live_passes(&uses, &dependencies)?;

        let order: Vec<PassId> = schedule.into_iter().filter(|&pass| live[pass]).map(PassId).collect();
        let culled = (0..self.passes().len()).filter(|&pass| !live[pass]).map(PassId).collect();

        let mut positions = vec![None; self.passes().len()];
        for (position, pass) in order.iter().enumerate() {
            positions[pass.0] = Some(position);
        }

        let mut live_dependencies: Vec<Dependency> = dependencies
            .into_iter()
            .filter(|dependency| live[dependency.before.0] && live[dependency.after.0])
            .collect();
        live_dependencies.sort_unstable_by_key(|dependency| {
            (
                positions[dependency.after.0],
                positions[dependency.before.0],
                dependency.resource,
            )
        });

        let lifetimes = self.lifetimes(&order);
        let alias_groups = alias_groups(self.resources(), &lifetimes);
        Ok(CompiledGraph {
            graph: self,
            order,
            culled,
            positions,
            dependencies: live_dependencies,
            lifetimes,
            alias_groups,
        })
    }

    fn uses(&self) -> Uses {
        let resource_count = self.resources().len();
        let mut uses = Uses {
            writers: vec![Vec::new(); resource_count],
            readers: vec![Vec::new(); resource_count],
        };
        for (pass_index, pass) in self.passes().iter().enumerate() {
            for input in &pass.inputs {
                uses.readers[input.resource.0].push(pass_index);
            }

            for (output_index, output) in pass.outputs.iter().enumerate() {
                uses.writers[output.resource.0].push((pass_index, output_index));
            }
        }

        uses
    }

    fn check_contents(&self, uses: &Uses) -> Result<(), GraphError> {
        let passes = self.passes();
        for (index, resource) in self.resources().iter().enumerate() {
            let writers = &uses.writers[index];
            let readers = &uses.readers[index];
            let transient = resource.origin == ResourceOrigin::Transient;
            if transient && writers.is_empty() && readers.is_empty() {
                return Err(GraphError::UnusedResource {
                    resource: resource.name.clone(),
                });
            }

            for (rank, &(pass, output)) in writers.iter().enumerate() {
                if passes[pass].outputs[output].load != LoadOp::Load {
                    continue;
                }

                if rank == 0 {
                    if transient {
                        return Err(GraphError::LoadWithoutContents {
                            pass: passes[pass].name().to_owned(),
                            resource: resource.name.clone(),
                        });
                    }

                    continue;
                }

                let (previous, previous_output) = writers[rank - 1];
                if passes[previous].outputs[previous_output].store == StoreOp::Discard {
                    return Err(GraphError::ReadDiscarded {
                        pass: passes[pass].name().to_owned(),
                        resource: resource.name.clone(),
                        writer: passes[previous].name().to_owned(),
                    });
                }
            }

            let Some(&first_reader) = readers.first() else {
                continue;
            };

            let Some(&(last_writer, last_output)) = writers.last() else {
                if transient {
                    return Err(GraphError::ReadWithoutWriter {
                        pass: passes[first_reader].name().to_owned(),
                        resource: resource.name.clone(),
                    });
                }

                continue;
            };

            if passes[last_writer].outputs[last_output].store == StoreOp::Discard {
                return Err(GraphError::ReadDiscarded {
                    pass: passes[first_reader].name().to_owned(),
                    resource: resource.name.clone(),
                    writer: passes[last_writer].name().to_owned(),
                });
            }
        }

        Ok(())
    }

    fn dependencies(&self, uses: &Uses) -> Vec<Dependency> {
        let passes = self.passes();
        let mut dependencies = Vec::new();
        for (index, writers) in uses.writers.iter().enumerate() {
            let resource = ResourceId(index);
            for pair in writers.windows(2) {
                let (before, _) = pair[0];
                let (after, output) = pair[1];
                let kind = if passes[after].outputs[output].load == LoadOp::Load {
                    DependencyKind::LoadAfterWrite
                } else {
                    DependencyKind::WriteAfterWrite
                };
                dependencies.push(Dependency {
                    before: PassId(before),
                    after: PassId(after),
                    resource,
                    kind,
                });
            }

            let Some(&(last_writer, _)) = writers.last() else {
                continue;
            };

            for &reader in &uses.readers[index] {
                dependencies.push(Dependency {
                    before: PassId(last_writer),
                    after: PassId(reader),
                    resource,
                    kind: DependencyKind::ReadAfterWrite,
                });
            }
        }

        dependencies
    }

    /// Kahn's algorithm, always taking the earliest-registered ready pass.
    fn schedule(&self, dependencies: &[Dependency]) -> Result<Vec<usize>, GraphError> {
        let pass_count = self.passes().len();
        let mut successors = vec![Vec::new(); pass_count];
        let mut waiting = vec![0usize; pass_count];
        for dependency in dependencies {
            successors[dependency.before.0].push(dependency.after.0);
            waiting[dependency.after.0] += 1;
        }

        let mut ready: BinaryHeap<Reverse<usize>> =
            (0..pass_count).filter(|&pass| waiting[pass] == 0).map(Reverse).collect();
        let mut order = Vec::with_capacity(pass_count);
        while let Some(Reverse(pass)) = ready.pop() {
            order.push(pass);
            for &successor in &successors[pass] {
                waiting[successor] -= 1;
                if waiting[successor] == 0 {
                    ready.push(Reverse(successor));
                }
            }
        }

        if order.len() == pass_count {
            return Ok(order);
        }

        Err(GraphError::Cycle {
            passes: self.cycle(dependencies, &waiting),
        })
    }

    /// Names one cycle among the passes Kahn's algorithm couldn't schedule.
    ///
    /// Every unscheduled pass still waits on at least one unscheduled pass, so walking from one to
    /// the pass it waits on never runs out and must revisit a pass.
    fn cycle(&self, dependencies: &[Dependency], waiting: &[usize]) -> Vec<String> {
        let pass_count = self.passes().len();
        let mut waits_on = vec![Vec::new(); pass_count];
        for dependency in dependencies {
            if waiting[dependency.before.0] > 0 && waiting[dependency.after.0] > 0 {
                waits_on[dependency.after.0].push(dependency.before.0);
            }
        }

        let mut path_position = vec![None; pass_count];
        let mut path = Vec::new();
        let mut current = (0..pass_count)
            .find(|&pass| waiting[pass] > 0)
            .expect("a failed schedule leaves a pass waiting");
        let start = loop {
            if let Some(position) = path_position[current] {
                break position;
            }

            path_position[current] = Some(path.len());
            path.push(current);
            current = *waits_on[current]
                .iter()
                .min()
                .expect("an unscheduled pass waits on an unscheduled pass");
        };

        // The walk went from each pass to the one it waits on; execution order is the reverse.
        let mut cycle = path.split_off(start);
        cycle.reverse();
        let earliest = cycle
            .iter()
            .enumerate()
            .min_by_key(|&(_, &pass)| pass)
            .map_or(0, |(position, _)| position);
        cycle.rotate_left(earliest);
        cycle.push(cycle[0]);
        cycle.into_iter().map(|pass| self.passes()[pass].name().to_owned()).collect()
    }

    fn live_passes(&self, uses: &Uses, dependencies: &[Dependency]) -> Result<Vec<bool>, GraphError> {
        let passes = self.passes();
        let mut live = vec![false; passes.len()];
        let mut worklist = Vec::new();
        for (index, resource) in self.resources().iter().enumerate() {
            if resource.origin != ResourceOrigin::Imported {
                continue;
            }

            let Some(&(writer, output)) = uses.writers[index].last() else {
                continue;
            };

            if passes[writer].outputs[output].store == StoreOp::Discard || live[writer] {
                continue;
            }

            live[writer] = true;
            worklist.push(writer);
        }

        let mut sources = vec![Vec::new(); passes.len()];
        for dependency in dependencies.iter().filter(|dependency| dependency.kind.carries_contents()) {
            sources[dependency.after.0].push(dependency.before.0);
        }

        while let Some(pass) = worklist.pop() {
            for &source in &sources[pass] {
                if live[source] {
                    continue;
                }

                live[source] = true;
                worklist.push(source);
            }
        }

        if !live.contains(&true) {
            return Err(GraphError::NoImportedOutput {
                graph: self.name().to_owned(),
            });
        }

        Ok(live)
    }

    fn lifetimes(&self, order: &[PassId]) -> Vec<Option<Lifetime>> {
        let mut lifetimes: Vec<Option<Lifetime>> = vec![None; self.resources().len()];
        for (position, pass) in order.iter().enumerate() {
            let pass = &self.passes()[pass.0];
            let inputs = pass.inputs.iter().map(|input| input.resource);
            let outputs = pass.outputs.iter().map(|output| output.resource);
            for resource in inputs.chain(outputs) {
                let lifetime = &mut lifetimes[resource.0];
                let Some(lifetime) = lifetime else {
                    *lifetime = Some(Lifetime {
                        first: position,
                        last: position,
                    });
                    continue;
                };

                lifetime.last = position;
            }
        }

        lifetimes
    }
}

/// First-fit over transient resources in order of first use: each joins the earliest group with
/// the same descriptor whose last member is done before it starts.
///
/// The descriptor is a target's format and size, or a buffer's size, because that is everything
/// an allocation depends on today. Anything that later changes one (sample count, mip levels,
/// array layers) must join [`ResourceDesc`](crate::render::ResourceDesc) when it's added, or two resources the backend can't
/// share memory between will be put in one group.
fn alias_groups(resources: &[Resource], lifetimes: &[Option<Lifetime>]) -> Vec<AliasGroup> {
    let mut candidates: Vec<(Lifetime, usize)> = lifetimes
        .iter()
        .enumerate()
        .filter(|&(index, _)| resources[index].origin == ResourceOrigin::Transient)
        .filter_map(|(index, lifetime)| Some(((*lifetime)?, index)))
        .collect();
    candidates.sort_unstable_by_key(|&(lifetime, index)| (lifetime.first, index));

    let mut groups: Vec<AliasGroup> = Vec::new();
    let mut group_ends: Vec<usize> = Vec::new();
    for (lifetime, index) in candidates {
        let resource = &resources[index];
        let fitting = groups.iter().zip(&group_ends).position(|(group, &end)| {
            let member = &resources[group.resources[0].0];
            end < lifetime.first && member.desc == resource.desc
        });

        let Some(fitting) = fitting else {
            groups.push(AliasGroup {
                resources: vec![ResourceId(index)],
            });
            group_ends.push(lifetime.last);
            continue;
        };

        groups[fitting].resources.push(ResourceId(index));
        group_ends[fitting] = lifetime.last;
    }

    groups
}
