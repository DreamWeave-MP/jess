//! Draw sets: retained draws, each a pipeline, what it binds, its buffers, and how much to draw,
//! that a plan's draw passes take by their tags every frame without being told again.
//!
//! A draw set binds its own material and draw groups; its pass group is the pass's, so one camera
//! block serves every set a pass draws. A draw set is checked once, when it is made, as a pass
//! checks a draw. What changes afterwards is its range (how many vertices and instances, and
//! where in its buffers they start), which changes whole, so a frame never draws half of one
//! change, and whether it is drawn at all.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::pass::{buffer_problem, group_problem};
use super::{BindGroup, Buffer, Draw, GpuError, IndexFormat, RenderPipeline};
use crate::shader::Frequency;

/// How much of its buffers a draw set draws, and from where: what changes in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawRange {
    /// How many vertices, or indices, to draw.
    pub count: u32,
    /// How many instances; none draws nothing.
    pub instance_count: u32,
    /// The first vertex drawn, or the first index of an indexed draw.
    pub first: u32,
    /// What is added to every index before it names a vertex; an indexed draw's only.
    pub base_vertex: i32,
    /// How many bytes into its vertex buffer its first vertex is: a multiple of 4.
    pub vertices_offset: u64,
    /// How many bytes into its instance buffer its first instance is: a multiple of 4.
    pub instances_offset: u64,
}

impl DrawRange {
    /// `count` vertices, or indices, of one instance, from the start of every buffer.
    #[must_use]
    pub fn new(count: u32) -> DrawRange {
        DrawRange {
            count,
            instance_count: 1,
            first: 0,
            base_vertex: 0,
            vertices_offset: 0,
            instances_offset: 0,
        }
    }
}

/// What a draw set draws.
#[derive(Clone, Debug)]
pub struct DrawSetDesc {
    /// The pipeline.
    pub pipeline: Arc<RenderPipeline>,
    /// The material's bind group, if its program binds anything per material.
    pub material: Option<Arc<BindGroup>>,
    /// The draw's bind group, if its program binds anything per draw.
    pub draw: Option<Arc<BindGroup>>,
    /// The per-vertex attributes, if the program declares any.
    pub vertices: Option<Arc<Buffer>>,
    /// The per-instance attributes, if the program declares any.
    pub instances: Option<Arc<Buffer>>,
    /// The indices of the vertices to draw, if the draw is indexed.
    pub indices: Option<(Arc<Buffer>, IndexFormat)>,
    /// How much of them it draws, and from where.
    pub range: DrawRange,
    /// Where the counts come from instead, written by the GPU, as a [`Draw`]'s `indirect` says.
    pub indirect: Option<(Arc<Buffer>, u64)>,
    /// What the passes that take it look for.
    pub tags: Vec<String>,
}

/// A retained draw.
#[derive(Debug)]
pub struct DrawSet {
    pipeline: Arc<RenderPipeline>,
    material: Option<Arc<BindGroup>>,
    draw: Option<Arc<BindGroup>>,
    vertices: Option<Arc<Buffer>>,
    instances: Option<Arc<Buffer>>,
    indices: Option<(Arc<Buffer>, IndexFormat)>,
    indirect: Option<(Arc<Buffer>, u64)>,
    tags: Box<[String]>,
    range: Mutex<DrawRange>,
    visible: AtomicBool,
}

impl DrawSet {
    /// A visible draw set of `desc`.
    ///
    /// # Errors
    ///
    /// What a pass refuses in a draw: a material or draw group missing where the program binds
    /// one, given where it doesn't, or laid out for another program; a buffer missing, unneeded,
    /// of the wrong usage, or too small for the range drawn from it.
    pub fn new(desc: DrawSetDesc) -> Result<DrawSet, GpuError> {
        let set = DrawSet {
            pipeline: desc.pipeline,
            material: desc.material,
            draw: desc.draw,
            vertices: desc.vertices,
            instances: desc.instances,
            indices: desc.indices,
            indirect: desc.indirect,
            tags: desc.tags.into_boxed_slice(),
            range: Mutex::new(desc.range),
            visible: AtomicBool::new(true),
        };
        set.check(desc.range)?;
        Ok(set)
    }

    /// Whether it can draw `range`: what [`DrawSet::set_range`] checks.
    ///
    /// # Errors
    ///
    /// As that.
    pub fn check(&self, range: DrawRange) -> Result<(), GpuError> {
        let draw = self.drawing(None, range);
        group_problem(&draw, &[Frequency::Material, Frequency::Draw])
            .or_else(|| buffer_problem(&draw))
            .map_or(Ok(()), |problem| {
                Err(GpuError::Invalid(format!("the draw set is invalid: {problem}")))
            })
    }

    /// The draw it is now, in a pass whose pass group is `pass`.
    #[must_use]
    pub fn draw<'a>(&'a self, pass: Option<&'a BindGroup>) -> Draw<'a> {
        self.drawing(pass, self.range())
    }

    /// The draw of `range` of it, in a pass whose pass group is `pass`.
    fn drawing<'a>(&'a self, pass: Option<&'a BindGroup>, range: DrawRange) -> Draw<'a> {
        Draw {
            pipeline: &self.pipeline,
            groups: [pass, self.material.as_deref(), self.draw.as_deref()],
            vertices: self.vertices.as_deref(),
            instances: self.instances.as_deref(),
            indices: self.indices.as_ref().map(|(buffer, format)| (&**buffer, *format)),
            count: range.count,
            instance_count: range.instance_count,
            first: range.first,
            base_vertex: range.base_vertex,
            vertices_offset: range.vertices_offset,
            instances_offset: range.instances_offset,
            indirect: self.indirect.as_ref().map(|(buffer, offset)| (&**buffer, *offset)),
        }
    }

    /// Its pipeline.
    #[must_use]
    pub fn pipeline(&self) -> &Arc<RenderPipeline> {
        &self.pipeline
    }

    /// What the passes that take it look for.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Whether it carries any of `tags`.
    #[must_use]
    pub fn matches(&self, tags: &[String]) -> bool {
        self.tags.iter().any(|tag| tags.contains(tag))
    }

    /// How much of its buffers it draws, and from where.
    #[must_use]
    pub fn range(&self) -> DrawRange {
        *self.range.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Draws `range` from the next frame on: every part of it at once.
    ///
    /// # Errors
    ///
    /// A range that runs past its vertex, instance, or index buffer, an offset that isn't a
    /// multiple of 4 inside its buffer, a base vertex without indices, or a first vertex or base
    /// vertex on an indirect draw, whose arguments hold them.
    pub fn set_range(&self, range: DrawRange) -> Result<(), GpuError> {
        self.change(|_| range)
    }

    /// Draws what `change` makes of its range, read and replaced as one step, so two changes to
    /// different parts of it from two threads both happen.
    fn change(&self, change: impl FnOnce(DrawRange) -> DrawRange) -> Result<(), GpuError> {
        let mut range = self.range.lock().unwrap_or_else(PoisonError::into_inner);
        let changed = change(*range);
        self.check(changed)?;
        *range = changed;
        drop(range);
        Ok(())
    }

    /// How many vertices, or indices, it draws.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.range().count
    }

    /// Draws `count` vertices, or indices, from the next frame on.
    ///
    /// # Errors
    ///
    /// A count that runs past its vertex or index buffer.
    pub fn set_count(&self, count: u32) -> Result<(), GpuError> {
        self.change(|range| DrawRange { count, ..range })
    }

    /// How many instances it draws.
    #[must_use]
    pub fn instance_count(&self) -> u32 {
        self.range().instance_count
    }

    /// Draws `instance_count` instances from the next frame on; none draws nothing.
    ///
    /// # Errors
    ///
    /// More than its instance buffer holds.
    pub fn set_instance_count(&self, instance_count: u32) -> Result<(), GpuError> {
        self.change(|range| DrawRange { instance_count, ..range })
    }

    /// Whether the passes that take it draw it.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    /// Shows or hides it from the next frame on.
    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Relaxed);
    }
}

/// The draw sets a plan's passes choose from, in the order they were added. A run takes them from
/// one collection; an application keeps as many as it has scenes.
#[derive(Debug)]
pub struct DrawSets {
    sets: Mutex<Vec<Arc<DrawSet>>>,
    version: AtomicU64,
    id: u64,
}

/// The next collection's identity.
static NEXT_SETS: AtomicU64 = AtomicU64::new(1);

impl Default for DrawSets {
    fn default() -> DrawSets {
        DrawSets {
            sets: Mutex::default(),
            version: AtomicU64::new(0),
            id: NEXT_SETS.fetch_add(1, Ordering::Relaxed),
        }
    }
}

impl DrawSets {
    /// An empty list.
    #[must_use]
    pub fn new() -> DrawSets {
        DrawSets::default()
    }

    /// Its identity, which no other collection ever has: with [`Self::version`], what a choice
    /// cached from it keys on.
    #[must_use]
    pub fn id(&self) -> u64 {
        self.id
    }

    /// Adds `set`, to be drawn from the next frame on; adding it again does nothing.
    pub fn add(&self, set: &Arc<DrawSet>) {
        let mut sets = self.sets.lock().unwrap_or_else(PoisonError::into_inner);
        if sets.iter().any(|held| Arc::ptr_eq(held, set)) {
            return;
        }
        sets.push(set.clone());
        drop(sets);
        self.version.fetch_add(1, Ordering::Relaxed);
    }

    /// Removes `set`; whether it was there.
    pub fn remove(&self, set: &Arc<DrawSet>) -> bool {
        let mut sets = self.sets.lock().unwrap_or_else(PoisonError::into_inner);
        let before = sets.len();
        sets.retain(|held| !Arc::ptr_eq(held, set));
        let removed = sets.len() != before;
        drop(sets);
        if removed {
            self.version.fetch_add(1, Ordering::Relaxed);
        }
        removed
    }

    /// Whether it holds `set`.
    #[must_use]
    pub fn contains(&self, set: &Arc<DrawSet>) -> bool {
        let sets = self.sets.lock().unwrap_or_else(PoisonError::into_inner);
        sets.iter().any(|held| Arc::ptr_eq(held, set))
    }

    /// How many sets it holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sets.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    /// Whether it holds none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Counts every addition and removal, so whatever caches a choice of sets knows when to make
    /// it again.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    /// The visible sets carrying any of `tags`, in the order they were added.
    #[must_use]
    pub fn matching(&self, tags: &[String]) -> Vec<Arc<DrawSet>> {
        let mut matched = self.tagged(tags);
        matched.retain(|set| set.visible());
        matched
    }

    /// The sets carrying any of `tags`, shown or hidden, in the order they were added: a choice
    /// that holds until the next addition or removal, whoever shows or hides what.
    #[must_use]
    pub fn tagged(&self, tags: &[String]) -> Vec<Arc<DrawSet>> {
        let sets = self.sets.lock().unwrap_or_else(PoisonError::into_inner);
        sets.iter().filter(|set| set.matches(tags)).cloned().collect()
    }
}
