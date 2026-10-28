//! Graph registration and the checks each registration runs.

use std::collections::HashMap;
use std::fmt;

use crate::render::error::GraphError;
use crate::render::pass::{Attachment, ClearValue, LoadOp, Pass, PassKind, PassOutput, ReadUsage, Subresource};
use crate::render::target::{BufferSize, Extent, TargetSize, TextureFormat};

/// A pass in one [`RenderGraph`]. Only meaningful for the graph that returned it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PassId(pub(crate) usize);

impl PassId {
    /// The pass's registration index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

impl fmt::Display for PassId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "pass #{}", self.0)
    }
}

/// A target or buffer in one [`RenderGraph`]. Only meaningful for the graph that returned it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ResourceId(pub(crate) usize);

impl ResourceId {
    /// The resource's registration index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "resource #{}", self.0)
    }
}

/// Whether a resource is a color target, a depth target, or a buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ResourceKind {
    /// A color target: written through color attachments, sampled as a color texture.
    RenderTarget,
    /// A depth target: written through the depth attachment, read for depth testing or sampled
    /// as a depth texture.
    DepthTarget,
    /// Bytes: written by compute passes, read by shaders, drawn from, and copied into and out of.
    Buffer,
}

impl ResourceKind {
    /// The kind's name, as errors print it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RenderTarget => "render target",
            Self::DepthTarget => "depth target",
            Self::Buffer => "buffer",
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(self.name())
    }
}

/// Who owns a resource's memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ResourceOrigin {
    /// The graph. The resource exists only between its first and last use, holds nothing before
    /// its first write, and may share memory with other transient resources of its kind.
    Transient,
    /// The host, such as the window's backbuffer. Its contents exist before the graph runs and
    /// are observed after it; it never shares memory.
    Imported,
    /// The plan that runs the graph: it holds what the last run left, the first run zeros, and
    /// is observed after each, the way the host reads it back from the plan; it never shares
    /// memory. A history target, say, or a buffer of what a frame found.
    Kept,
}

impl ResourceOrigin {
    /// The origin's name, as the debug dump prints it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Imported => "imported",
            Self::Kept => "kept",
        }
    }
}

/// What a target is made as: a format, a size, and how many mip levels, array layers, and samples
/// a pixel it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetDesc {
    /// Pixel format: a depth target's is a depth format.
    pub format: TextureFormat,
    /// Size of level 0, fixed or relative to the viewport.
    pub size: TargetSize,
    /// Mip levels, each half the one before: at least 1, at most as many as the size halves to.
    pub mips: u32,
    /// Array layers: at least 1.
    pub layers: u32,
    /// Samples a pixel: 1, or 4 for a multisampled target, which has one level and one layer, is
    /// drawn into and resolved, and never sampled or copied.
    pub samples: u32,
}

impl TargetDesc {
    /// One level, one layer, one sample of `format` at `size`.
    #[must_use]
    pub const fn new(format: TextureFormat, size: TargetSize) -> Self {
        Self {
            format,
            size,
            mips: 1,
            layers: 1,
            samples: 1,
        }
    }

    /// The same target with `mips` levels.
    #[must_use]
    pub const fn mips(self, mips: u32) -> Self {
        Self { mips, ..self }
    }

    /// The same target with `layers` layers.
    #[must_use]
    pub const fn layers(self, layers: u32) -> Self {
        Self { layers, ..self }
    }

    /// The same target with `samples` samples a pixel.
    #[must_use]
    pub const fn samples(self, samples: u32) -> Self {
        Self { samples, ..self }
    }

    /// What is wrong with its shape, if anything.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !self.size.is_valid() {
            return Some(format!("its size, {}, is zero, negative, or not finite", self.size));
        }
        if self.mips == 0 || self.layers == 0 {
            return Some("at least one mip level and one layer".to_owned());
        }
        if !matches!(self.samples, 1 | 4) {
            return Some(format!("1 or 4 samples a pixel, not {}", self.samples));
        }
        if self.samples > 1 && (self.mips > 1 || self.layers > 1) {
            return Some("one level and one layer, being multisampled".to_owned());
        }
        let most = match self.size {
            TargetSize::Absolute { width, height } => 32 - width.max(height).leading_zeros(),
            TargetSize::Relative { .. } => 32,
        };
        if self.mips > most {
            return Some(format!("at most {most} mip levels at {}, not {}", self.size, self.mips));
        }
        None
    }

    /// The size of level 0 at `viewport`. A relative size with several levels never resolves
    /// smaller than they need, 2^(levels - 1) pixels each way, so every level exists however
    /// small the viewport.
    #[must_use]
    pub fn resolve(&self, viewport: Extent) -> Extent {
        let size = self.size.resolve(viewport);
        if matches!(self.size, TargetSize::Absolute { .. }) {
            return size;
        }

        let floor = self.floor();
        Extent {
            width: size.width.max(floor),
            height: size.height.max(floor),
        }
    }

    /// The size of `level` at `viewport`, or `None` for a relative size without one.
    #[must_use]
    pub fn extent(&self, level: u32, viewport: Option<Extent>) -> Option<Extent> {
        match (self.size, viewport) {
            (TargetSize::Absolute { width, height }, _) => Some(Extent { width, height }.level(level)),
            (_, Some(viewport)) => Some(self.resolve(viewport).level(level)),
            (TargetSize::Relative { .. }, None) => None,
        }
    }

    /// What decides the size of `level`: two levels of targets with the same one are the same
    /// size at every viewport.
    pub(crate) fn level_size(&self, level: u32) -> LevelSize {
        match self.size {
            TargetSize::Absolute { width, height } => LevelSize::Fixed(Extent { width, height }.level(level)),
            TargetSize::Relative { scale } => LevelSize::Relative {
                scale,
                level,
                floor: self.floor(),
            },
        }
    }

    /// The fewest pixels level 0 of a relative size has each way: what its last level needs.
    const fn floor(&self) -> u32 {
        1 << (self.mips - 1)
    }
}

/// A level's size, as far as a graph knows it before a viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LevelSize {
    /// Fixed pixels.
    Fixed(Extent),
    /// A level of the viewport times a scale, level 0 never under `floor` pixels each way.
    Relative { scale: f32, level: u32, floor: u32 },
}

/// What a resource is made as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResourceDesc {
    /// A render or depth target: a depth target exactly when its format is a depth format.
    Texture(TargetDesc),
    /// A buffer of a fixed size, or one sized by the viewport.
    Buffer(BufferSize),
}

/// A registered target or buffer.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    /// The resource's name, unique within its graph.
    pub name: String,
    /// What it is made as.
    pub desc: ResourceDesc,
    /// Whether the graph or the host owns it.
    pub origin: ResourceOrigin,
}

impl Resource {
    /// Color target, depth target, or buffer.
    #[must_use]
    pub const fn kind(&self) -> ResourceKind {
        match self.desc {
            ResourceDesc::Texture(target) if target.format.is_depth() => ResourceKind::DepthTarget,
            ResourceDesc::Texture(_) => ResourceKind::RenderTarget,
            ResourceDesc::Buffer(_) => ResourceKind::Buffer,
        }
    }

    /// Whether what it holds is seen after a run: an imported or kept resource's is.
    #[must_use]
    pub const fn is_observed(&self) -> bool {
        !matches!(self.origin, ResourceOrigin::Transient)
    }

    /// A target's pixel format; `None` for a buffer.
    #[must_use]
    pub const fn format(&self) -> Option<TextureFormat> {
        match self.desc {
            ResourceDesc::Texture(target) => Some(target.format),
            ResourceDesc::Buffer(_) => None,
        }
    }

    /// A target's size; `None` for a buffer.
    #[must_use]
    pub const fn size(&self) -> Option<TargetSize> {
        match self.desc {
            ResourceDesc::Texture(target) => Some(target.size),
            ResourceDesc::Buffer(_) => None,
        }
    }

    /// A target's description; `None` for a buffer.
    #[must_use]
    pub const fn target(&self) -> Option<TargetDesc> {
        match self.desc {
            ResourceDesc::Texture(target) => Some(target),
            ResourceDesc::Buffer(_) => None,
        }
    }

    /// How many subresources it has: a target's levels times its layers, and 1 for a buffer.
    #[must_use]
    pub const fn subresources(&self) -> u32 {
        match self.desc {
            ResourceDesc::Texture(target) => target.mips * target.layers,
            ResourceDesc::Buffer(_) => 1,
        }
    }

    /// A target's samples a pixel; 1 for a buffer.
    #[must_use]
    pub const fn samples(&self) -> u32 {
        match self.desc {
            ResourceDesc::Texture(target) => target.samples,
            ResourceDesc::Buffer(_) => 1,
        }
    }

    /// Whether `subresource` is one of its own.
    #[must_use]
    pub const fn has(&self, subresource: Subresource) -> bool {
        match self.desc {
            ResourceDesc::Texture(target) => subresource.level < target.mips && subresource.layer < target.layers,
            ResourceDesc::Buffer(_) => subresource.level == 0 && subresource.layer == 0,
        }
    }

    /// A buffer's size in bytes, if it doesn't depend on the viewport; `None` for a target.
    #[must_use]
    pub const fn bytes(&self) -> Option<u64> {
        match self.desc {
            ResourceDesc::Texture(_) => None,
            ResourceDesc::Buffer(size) => size.fixed(),
        }
    }

    /// A buffer's size; `None` for a target.
    #[must_use]
    pub const fn buffer_size(&self) -> Option<BufferSize> {
        match self.desc {
            ResourceDesc::Texture(_) => None,
            ResourceDesc::Buffer(size) => Some(size),
        }
    }

    /// A buffer's size in bytes at `viewport`, or without one if it doesn't need one; `None` for
    /// a target, and for a buffer sized by the viewport without one.
    #[must_use]
    pub fn bytes_at(&self, viewport: Option<Extent>) -> Option<u64> {
        match (self.desc, viewport) {
            (ResourceDesc::Texture(_), _) => None,
            (ResourceDesc::Buffer(size), None) => size.fixed(),
            (ResourceDesc::Buffer(size), Some(viewport)) => Some(size.resolve(viewport)),
        }
    }
}

/// Targets and the passes that use them, before ordering.
///
/// Registration checks everything one resource or one pass can get wrong on its own;
/// [`RenderGraph::compile`] checks the rest.
#[derive(Clone, Debug, Default)]
pub struct RenderGraph {
    name: String,
    resources: Vec<Resource>,
    passes: Vec<Pass>,
    resource_ids: HashMap<String, ResourceId>,
    pass_ids: HashMap<String, PassId>,
}

impl RenderGraph {
    /// An empty graph called `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    /// The graph's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Registers a transient color target.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, the format is the wrong kind, or the size is invalid.
    pub fn add_render_target(
        &mut self,
        name: impl Into<String>,
        format: TextureFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_target_of(
            name.into(),
            ResourceKind::RenderTarget,
            format,
            size,
            ResourceOrigin::Transient,
        )
    }

    /// Registers a transient depth target.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, the format is the wrong kind, or the size is invalid.
    pub fn add_depth_target(
        &mut self,
        name: impl Into<String>,
        format: TextureFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_target_of(
            name.into(),
            ResourceKind::DepthTarget,
            format,
            size,
            ResourceOrigin::Transient,
        )
    }

    /// Registers a color target the host owns, such as the window's backbuffer.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, the format is the wrong kind, or the size is invalid.
    pub fn import_render_target(
        &mut self,
        name: impl Into<String>,
        format: TextureFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_target_of(
            name.into(),
            ResourceKind::RenderTarget,
            format,
            size,
            ResourceOrigin::Imported,
        )
    }

    /// Registers a depth target the host owns.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, the format is the wrong kind, or the size is invalid.
    pub fn import_depth_target(
        &mut self,
        name: impl Into<String>,
        format: TextureFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_target_of(name.into(), ResourceKind::DepthTarget, format, size, ResourceOrigin::Imported)
    }

    /// Registers a transient target of any shape: levels, layers, and samples.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or the shape is invalid.
    pub fn add_target(&mut self, name: impl Into<String>, target: TargetDesc) -> Result<ResourceId, GraphError> {
        self.register_target(name.into(), None, target, ResourceOrigin::Transient)
    }

    /// Registers a target of any shape that the host owns.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or the shape is invalid.
    pub fn import_target(&mut self, name: impl Into<String>, target: TargetDesc) -> Result<ResourceId, GraphError> {
        self.register_target(name.into(), None, target, ResourceOrigin::Imported)
    }

    /// Registers a transient buffer of `bytes` bytes.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or `bytes` isn't a nonzero multiple of 4.
    pub fn add_buffer(&mut self, name: impl Into<String>, bytes: u64) -> Result<ResourceId, GraphError> {
        self.add_resource(
            name.into(),
            ResourceDesc::Buffer(BufferSize::Bytes(bytes)),
            ResourceOrigin::Transient,
        )
    }

    /// Registers a resource of any description and origin: a target of any shape, a buffer of
    /// any size, owned by the graph, the host, or the plan that runs it.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or the description is invalid.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        desc: ResourceDesc,
        origin: ResourceOrigin,
    ) -> Result<ResourceId, GraphError> {
        match desc {
            ResourceDesc::Texture(target) => self.register_target(name.into(), None, target, origin),
            ResourceDesc::Buffer(_) => self.add_resource(name.into(), desc, origin),
        }
    }

    /// Registers a buffer of `bytes` bytes the host owns: one read back after the graph runs, say,
    /// or one draw sets draw from.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or `bytes` isn't a nonzero multiple of 4.
    pub fn import_buffer(&mut self, name: impl Into<String>, bytes: u64) -> Result<ResourceId, GraphError> {
        self.add_resource(
            name.into(),
            ResourceDesc::Buffer(BufferSize::Bytes(bytes)),
            ResourceOrigin::Imported,
        )
    }

    /// Registers `target`, of `kind` where the caller named one.
    fn register_target(
        &mut self,
        name: String,
        kind: Option<ResourceKind>,
        target: TargetDesc,
        origin: ResourceOrigin,
    ) -> Result<ResourceId, GraphError> {
        self.check_name(&name)?;
        if let Some(kind) = kind
            && target.format.is_depth() != (kind == ResourceKind::DepthTarget)
        {
            return Err(GraphError::FormatKind {
                resource: name,
                format: target.format,
                kind,
            });
        }

        if !target.size.is_valid() {
            return Err(GraphError::InvalidSize {
                resource: name,
                size: target.size,
            });
        }

        if let Some(problem) = target.problem() {
            return Err(GraphError::InvalidTargetShape { resource: name, problem });
        }

        self.add_resource(name, ResourceDesc::Texture(target), origin)
    }

    fn add_target_of(
        &mut self,
        name: String,
        kind: ResourceKind,
        format: TextureFormat,
        size: TargetSize,
        origin: ResourceOrigin,
    ) -> Result<ResourceId, GraphError> {
        self.register_target(name, Some(kind), TargetDesc::new(format, size), origin)
    }

    fn check_name(&self, name: &str) -> Result<(), GraphError> {
        if name.is_empty() {
            return Err(GraphError::EmptyResourceName);
        }

        if self.resource_ids.contains_key(name) {
            return Err(GraphError::DuplicateResource {
                resource: name.to_owned(),
            });
        }

        Ok(())
    }

    fn add_resource(&mut self, name: String, desc: ResourceDesc, origin: ResourceOrigin) -> Result<ResourceId, GraphError> {
        self.check_name(&name)?;
        if let ResourceDesc::Buffer(size) = desc
            && let Some(problem) = size.problem()
        {
            return Err(match size {
                BufferSize::Bytes(bytes) => GraphError::InvalidBufferSize { resource: name, bytes },
                BufferSize::Image { .. } => GraphError::InvalidBufferShape { resource: name, problem },
            });
        }

        let id = ResourceId(self.resources.len());
        self.resource_ids.insert(name.clone(), id);
        self.resources.push(Resource { name, desc, origin });
        Ok(id)
    }

    /// Registers a pass.
    ///
    /// # Errors
    ///
    /// When the pass gets something wrong on its own: an empty or taken name, an empty owner, no
    /// outputs, a resource this graph never registered, a read or write its resource's kind or
    /// format or the pass's kind doesn't allow, a resource used twice, attachments that don't fit
    /// together (more than one depth attachment, or different sizes), or a copy that doesn't copy
    /// one resource into one it fits.
    pub fn add_pass(&mut self, pass: Pass) -> Result<PassId, GraphError> {
        self.check_pass(&pass)?;
        let id = PassId(self.passes.len());
        self.pass_ids.insert(pass.metadata.name.clone(), id);
        self.passes.push(pass);
        Ok(id)
    }

    /// Every registered target and buffer, indexed by [`ResourceId::index`].
    #[must_use]
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    /// Every registered pass, indexed by [`PassId::index`].
    #[must_use]
    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }

    /// The resource `id` names, or `None` for an id from another graph.
    #[must_use]
    pub fn resource(&self, id: ResourceId) -> Option<&Resource> {
        self.resources.get(id.0)
    }

    /// The pass `id` names, or `None` for an id from another graph.
    #[must_use]
    pub fn pass(&self, id: PassId) -> Option<&Pass> {
        self.passes.get(id.0)
    }

    /// The target or buffer called `name`.
    #[must_use]
    pub fn resource_id(&self, name: &str) -> Option<ResourceId> {
        self.resource_ids.get(name).copied()
    }

    /// The pass called `name`.
    #[must_use]
    pub fn pass_id(&self, name: &str) -> Option<PassId> {
        self.pass_ids.get(name).copied()
    }

    fn check_pass(&self, pass: &Pass) -> Result<(), GraphError> {
        let name = pass.name();
        if name.is_empty() {
            return Err(GraphError::EmptyPassName);
        }

        if pass.metadata.owner.is_empty() {
            return Err(GraphError::EmptyOwner { pass: name.to_owned() });
        }

        if self.pass_ids.contains_key(name) {
            return Err(GraphError::DuplicatePass { pass: name.to_owned() });
        }

        if pass.outputs.is_empty() {
            return Err(GraphError::NoOutputs { pass: name.to_owned() });
        }

        self.check_inputs(pass)?;
        self.check_outputs(pass)?;

        let writes: Vec<(ResourceId, Subresource)> = pass.writes().collect();
        for (index, &(id, subresource)) in writes.iter().enumerate() {
            let resource = &self.resources[id.0];
            if writes[..index].contains(&(id, subresource)) {
                return Err(GraphError::DuplicateAccess {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                });
            }

            if pass
                .inputs
                .iter()
                .any(|input| input.resource == id && overlap(input.one(), Some(subresource)))
            {
                return Err(GraphError::ReadWriteConflict {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                });
            }
        }

        if pass.kind == PassKind::Copy {
            return self.check_copy(pass);
        }

        if !pass.region.is_whole() {
            return Err(GraphError::RegionOutsideCopy { pass: name.to_owned() });
        }

        self.check_attachments(pass)
    }

    /// Whether each of `pass`'s reads is one its resource and its kind of pass allow, of a level and
    /// layer the resource has, and touching nothing another read does.
    fn check_inputs(&self, pass: &Pass) -> Result<(), GraphError> {
        let name = pass.name();
        // A pass touches a handful of resources, so the duplicate scans stay quadratic.
        for (index, input) in pass.inputs.iter().enumerate() {
            let resource = self.registered(name, input.resource)?;
            if !input.usage.allows(resource.kind()) {
                return Err(GraphError::IncompatibleRead {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    usage: input.usage,
                    kind: resource.kind(),
                });
            }

            if !pass.kind.reads(input.usage) {
                return Err(GraphError::PassCantRead {
                    pass: name.to_owned(),
                    kind: pass.kind,
                    usage: input.usage,
                });
            }

            if let Some(subresource) = input.subresource
                && !resource.has(subresource)
            {
                return Err(GraphError::SubresourceOutOfRange {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    subresource,
                });
            }

            if resource.samples() > 1 && input.usage != ReadUsage::DepthTest {
                return Err(GraphError::Multisampled {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    usage: input.usage.description(),
                });
            }

            let one = input.one();
            if pass.inputs[..index]
                .iter()
                .any(|earlier| earlier.resource == input.resource && overlap(earlier.one(), one))
            {
                return Err(GraphError::DuplicateAccess {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                });
            }
        }

        Ok(())
    }

    /// Whether each of `pass`'s writes is one its resource and its kind of pass allow, cleared to a
    /// value that fits, into a level and layer the resource has, resolving where it can.
    fn check_outputs(&self, pass: &Pass) -> Result<(), GraphError> {
        let name = pass.name();
        for output in &pass.outputs {
            let resource = self.registered(name, output.resource)?;
            if !output.attachment.allows(resource.kind()) {
                return Err(GraphError::IncompatibleWrite {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    attachment: output.attachment,
                    kind: resource.kind(),
                });
            }

            if !pass.kind.writes(output.attachment) {
                return Err(GraphError::PassCantWrite {
                    pass: name.to_owned(),
                    kind: pass.kind,
                    attachment: output.attachment,
                });
            }

            if let LoadOp::Clear(clear) = output.load {
                check_clear(name, resource, clear)?;
            }

            if !resource.has(output.subresource) {
                return Err(GraphError::SubresourceOutOfRange {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    subresource: output.subresource,
                });
            }

            let usage = match output.attachment {
                Attachment::Storage => Some("a storage texture"),
                Attachment::Copy => Some("a copy's destination"),
                Attachment::Color | Attachment::Depth => None,
            };
            if let Some(usage) = usage
                && resource.samples() > 1
            {
                return Err(GraphError::Multisampled {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    usage,
                });
            }

            if let Some((target, subresource)) = output.resolve {
                self.check_resolve(name, output, resource, target, subresource)?;
            }
        }

        Ok(())
    }

    /// Whether `output`, a write of `source`, can resolve into `subresource` of `target`.
    fn check_resolve(
        &self,
        pass: &str,
        output: &PassOutput,
        source: &Resource,
        target: ResourceId,
        subresource: Subresource,
    ) -> Result<(), GraphError> {
        let invalid = |problem: String| GraphError::InvalidResolve {
            pass: pass.to_owned(),
            resource: source.name.clone(),
            problem,
        };
        if output.attachment != Attachment::Color {
            return Err(invalid("only a color attachment resolves".to_owned()));
        }

        let into = self.registered(pass, target)?;
        let (Some(from), Some(to), ResourceKind::RenderTarget) = (source.target(), into.target(), into.kind()) else {
            return Err(invalid(format!("\"{}\" is a {}", into.name, into.kind())));
        };

        if from.samples == 1 {
            return Err(invalid("it has one sample, so there is nothing to resolve".to_owned()));
        }

        if to.samples > 1 {
            return Err(invalid(format!("\"{}\" is multisampled too", into.name)));
        }

        if from.format != to.format {
            return Err(invalid(format!(
                "it is {} and \"{}\" is {}",
                from.format, into.name, to.format
            )));
        }

        if !into.has(subresource) {
            return Err(GraphError::SubresourceOutOfRange {
                pass: pass.to_owned(),
                resource: into.name.clone(),
                subresource,
            });
        }

        if from.level_size(output.subresource.level) != to.level_size(subresource.level) {
            return Err(invalid(format!("\"{}\" is another size at {subresource}", into.name)));
        }

        Ok(())
    }

    /// Whether a drawing pass's attachments fit together: at most one depth attachment, and one
    /// size and one sample count among them all.
    fn check_attachments(&self, pass: &Pass) -> Result<(), GraphError> {
        let outputs = pass
            .outputs
            .iter()
            .filter(|output| matches!(output.attachment, Attachment::Color | Attachment::Depth))
            .map(|output| (output.resource, output.subresource, output.attachment == Attachment::Depth));
        let depth_tests = pass.inputs.iter().filter(|input| input.usage == ReadUsage::DepthTest);
        let attachments = outputs.chain(depth_tests.map(|input| (input.resource, input.one().unwrap_or_default(), true)));

        let mut depth: Option<&Resource> = None;
        let mut first: Option<(&Resource, LevelSize)> = None;
        for (id, subresource, is_depth) in attachments {
            let resource = &self.resources[id.0];
            if is_depth {
                if let Some(first) = depth {
                    return Err(GraphError::MultipleDepthAttachments {
                        pass: pass.name().to_owned(),
                        first: first.name.clone(),
                        second: resource.name.clone(),
                    });
                }

                depth = Some(resource);
            }

            let Some(target) = resource.target() else {
                continue;
            };

            let size = target.level_size(subresource.level);
            let Some((first, first_size)) = first else {
                first = Some((resource, size));
                continue;
            };

            if first_size != size {
                return Err(GraphError::AttachmentSizeMismatch {
                    pass: pass.name().to_owned(),
                    first: first.name.clone(),
                    second: resource.name.clone(),
                });
            }

            if first.samples() != resource.samples() {
                return Err(GraphError::SampleCountMismatch {
                    pass: pass.name().to_owned(),
                    first: first.name.clone(),
                    second: resource.name.clone(),
                });
            }
        }

        Ok(())
    }

    fn registered(&self, pass: &str, id: ResourceId) -> Result<&Resource, GraphError> {
        self.resources.get(id.0).ok_or_else(|| GraphError::UnknownResource {
            pass: pass.to_owned(),
            resource: id,
        })
    }
}

/// Whether `pass` may clear `resource` to `clear`: a value of its format's kind and in range for
/// a target, zero for a buffer.
fn check_clear(pass: &str, resource: &Resource, clear: ClearValue) -> Result<(), GraphError> {
    let Some(format) = resource.format() else {
        if clear == ClearValue::Zero {
            return Ok(());
        }

        return Err(GraphError::BufferClearValue {
            pass: pass.to_owned(),
            resource: resource.name.clone(),
            clear,
        });
    };

    if !clear.fits(format) {
        return Err(GraphError::ClearValueMismatch {
            pass: pass.to_owned(),
            resource: resource.name.clone(),
            format,
            clear,
        });
    }

    if !clear.is_valid() {
        return Err(GraphError::InvalidClearValue {
            pass: pass.to_owned(),
            resource: resource.name.clone(),
            clear,
        });
    }

    Ok(())
}

/// Whether two reads or writes of one resource touch a level of a layer in common: `None` is
/// every one.
fn overlap(first: Option<Subresource>, second: Option<Subresource>) -> bool {
    match (first, second) {
        (Some(first), Some(second)) => first == second,
        _ => true,
    }
}
