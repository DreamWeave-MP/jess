//! Draw sets: retained draws, each a pipeline, what it binds, its buffers, and how much to draw,
//! that a plan's draw passes take by their tags every frame without being told again.
//!
//! A draw set binds its own material and draw groups; its pass group is the pass's, so one camera
//! block serves every set a pass draws. A draw set is checked once, when it is made, as a pass
//! checks a draw. What changes afterwards (how many vertices and instances, whether it is drawn at
//! all) changes through atomics, so whatever draws the frame never waits on whoever changes it.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use super::pass::{buffer_problem, group_problem};
use super::{BindGroup, Buffer, Draw, GpuError, IndexFormat, RenderPipeline};
use crate::shader::Frequency;

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
    /// How many vertices, or indices, to draw.
    pub count: u32,
    /// How many instances.
    pub instance_count: u32,
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
    count: AtomicU32,
    instance_count: AtomicU32,
    visible: AtomicBool,
}

impl DrawSet {
    /// A visible draw set of `desc`.
    ///
    /// # Errors
    ///
    /// What a pass refuses in a draw: a material or draw group missing where the program binds
    /// one, given where it doesn't, or laid out for another program; a buffer missing, unneeded,
    /// of the wrong usage, or too small for the count; no instances.
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
            count: AtomicU32::new(desc.count),
            instance_count: AtomicU32::new(desc.instance_count),
            visible: AtomicBool::new(true),
        };
        set.check(desc.count, desc.instance_count)?;
        Ok(set)
    }

    /// Whether `count` vertices of `instance_count` instances of it can be drawn.
    fn check(&self, count: u32, instance_count: u32) -> Result<(), GpuError> {
        let draw = Draw {
            count,
            instance_count,
            ..self.draw(None)
        };
        let problem = if instance_count == 0 && self.indirect.is_none() {
            Some("at least one instance".to_owned())
        } else {
            group_problem(&draw, &[Frequency::Material, Frequency::Draw]).or_else(|| buffer_problem(&draw))
        };
        problem.map_or(Ok(()), |problem| {
            Err(GpuError::Invalid(format!("the draw set is invalid: {problem}")))
        })
    }

    /// The draw it is now, in a pass whose pass group is `pass`.
    #[must_use]
    pub fn draw<'a>(&'a self, pass: Option<&'a BindGroup>) -> Draw<'a> {
        Draw {
            pipeline: &self.pipeline,
            groups: [pass, self.material.as_deref(), self.draw.as_deref()],
            vertices: self.vertices.as_deref(),
            instances: self.instances.as_deref(),
            indices: self.indices.as_ref().map(|(buffer, format)| (&**buffer, *format)),
            count: self.count(),
            instance_count: self.instance_count(),
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

    /// How many vertices, or indices, it draws.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count.load(Ordering::Relaxed)
    }

    /// Draws `count` vertices, or indices, from the next frame on.
    ///
    /// # Errors
    ///
    /// A count that runs past its vertex or index buffer.
    pub fn set_count(&self, count: u32) -> Result<(), GpuError> {
        self.check(count, self.instance_count())?;
        self.count.store(count, Ordering::Relaxed);
        Ok(())
    }

    /// How many instances it draws.
    #[must_use]
    pub fn instance_count(&self) -> u32 {
        self.instance_count.load(Ordering::Relaxed)
    }

    /// Draws `instance_count` instances from the next frame on.
    ///
    /// # Errors
    ///
    /// None, or more than its instance buffer holds.
    pub fn set_instance_count(&self, instance_count: u32) -> Result<(), GpuError> {
        self.check(self.count(), instance_count)?;
        self.instance_count.store(instance_count, Ordering::Relaxed);
        Ok(())
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
