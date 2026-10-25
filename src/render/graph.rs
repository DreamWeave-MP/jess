//! Graph registration and the checks each registration runs.

use std::collections::HashMap;
use std::fmt;

use crate::render::error::GraphError;
use crate::render::pass::{Attachment, ClearValue, LoadOp, Pass, PassKind, ReadUsage};
use crate::render::target::{TargetSize, TextureFormat};

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
    /// Bytes: written by compute passes, read by shaders, drawn from, and copied.
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
}

impl ResourceOrigin {
    /// The origin's name, as the debug dump prints it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Transient => "transient",
            Self::Imported => "imported",
        }
    }
}

/// What a resource is made as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResourceDesc {
    /// A render or depth target: a depth target exactly when `format` is a depth format.
    Texture {
        /// Pixel format.
        format: TextureFormat,
        /// Size, fixed or relative to the viewport.
        size: TargetSize,
    },
    /// A buffer.
    Buffer {
        /// How many bytes it holds: a nonzero multiple of 4.
        bytes: u64,
    },
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
            ResourceDesc::Texture { format, .. } if format.is_depth() => ResourceKind::DepthTarget,
            ResourceDesc::Texture { .. } => ResourceKind::RenderTarget,
            ResourceDesc::Buffer { .. } => ResourceKind::Buffer,
        }
    }

    /// A target's pixel format; `None` for a buffer.
    #[must_use]
    pub const fn format(&self) -> Option<TextureFormat> {
        match self.desc {
            ResourceDesc::Texture { format, .. } => Some(format),
            ResourceDesc::Buffer { .. } => None,
        }
    }

    /// A target's size; `None` for a buffer.
    #[must_use]
    pub const fn size(&self) -> Option<TargetSize> {
        match self.desc {
            ResourceDesc::Texture { size, .. } => Some(size),
            ResourceDesc::Buffer { .. } => None,
        }
    }

    /// A buffer's size in bytes; `None` for a target.
    #[must_use]
    pub const fn bytes(&self) -> Option<u64> {
        match self.desc {
            ResourceDesc::Texture { .. } => None,
            ResourceDesc::Buffer { bytes } => Some(bytes),
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
        self.add_target(
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
        self.add_target(
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
        self.add_target(
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
        self.add_target(name.into(), ResourceKind::DepthTarget, format, size, ResourceOrigin::Imported)
    }

    /// Registers a transient buffer of `bytes` bytes.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or `bytes` isn't a nonzero multiple of 4.
    pub fn add_buffer(&mut self, name: impl Into<String>, bytes: u64) -> Result<ResourceId, GraphError> {
        self.add_resource(name.into(), ResourceDesc::Buffer { bytes }, ResourceOrigin::Transient)
    }

    /// Registers a buffer of `bytes` bytes the host owns: one read back after the graph runs, say,
    /// or one draw sets draw from.
    ///
    /// # Errors
    ///
    /// When the name is empty or taken, or `bytes` isn't a nonzero multiple of 4.
    pub fn import_buffer(&mut self, name: impl Into<String>, bytes: u64) -> Result<ResourceId, GraphError> {
        self.add_resource(name.into(), ResourceDesc::Buffer { bytes }, ResourceOrigin::Imported)
    }

    fn add_target(
        &mut self,
        name: String,
        kind: ResourceKind,
        format: TextureFormat,
        size: TargetSize,
        origin: ResourceOrigin,
    ) -> Result<ResourceId, GraphError> {
        self.check_name(&name)?;
        if format.is_depth() != (kind == ResourceKind::DepthTarget) {
            return Err(GraphError::FormatKind {
                resource: name,
                format,
                kind,
            });
        }

        if !size.is_valid() {
            return Err(GraphError::InvalidSize { resource: name, size });
        }

        self.add_resource(name, ResourceDesc::Texture { format, size }, origin)
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
        if let ResourceDesc::Buffer { bytes } = desc
            && (bytes == 0 || !bytes.is_multiple_of(4))
        {
            return Err(GraphError::InvalidBufferSize { resource: name, bytes });
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

            if pass.inputs[..index].iter().any(|earlier| earlier.resource == input.resource) {
                return Err(GraphError::DuplicateAccess {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                });
            }
        }

        for (index, output) in pass.outputs.iter().enumerate() {
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

            if pass.outputs[..index]
                .iter()
                .any(|earlier| earlier.resource == output.resource)
            {
                return Err(GraphError::DuplicateAccess {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                });
            }

            if pass.inputs.iter().any(|input| input.resource == output.resource) {
                return Err(GraphError::ReadWriteConflict {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                });
            }
        }

        if pass.kind == PassKind::Copy {
            return self.check_copy(pass);
        }

        self.check_attachments(pass)
    }

    /// Whether a copy pass copies one resource into one it fits.
    fn check_copy(&self, pass: &Pass) -> Result<(), GraphError> {
        let invalid = |problem: String| GraphError::InvalidCopy {
            pass: pass.name().to_owned(),
            problem,
        };
        let ([input], [output]) = (&pass.inputs[..], &pass.outputs[..]) else {
            return Err(invalid("expected one source and one destination".to_owned()));
        };

        if let LoadOp::Clear(_) = output.load {
            return Err(invalid(
                "the copy writes its destination, so clearing it first is wasted: load it or don't care".to_owned(),
            ));
        }

        let source = &self.resources[input.resource.0];
        let destination = &self.resources[output.resource.0];
        let problem = match (source.desc, destination.desc) {
            (ResourceDesc::Buffer { bytes: from }, ResourceDesc::Buffer { bytes: to }) if to < from => {
                format!(
                    "\"{}\" holds {from} bytes and \"{}\" only {to}",
                    source.name, destination.name
                )
            }
            (
                ResourceDesc::Texture { format, size },
                ResourceDesc::Texture {
                    format: into,
                    size: sized,
                },
            ) if (format, size) != (into, sized) => {
                format!(
                    "\"{}\" is {size} {format} and \"{}\" is {sized} {into}: expected the same format and size",
                    source.name, destination.name
                )
            }
            (ResourceDesc::Texture { format, .. }, ResourceDesc::Buffer { .. })
            | (ResourceDesc::Buffer { .. }, ResourceDesc::Texture { format, .. })
                if format.texel_bytes().is_none() =>
            {
                format!("{format} has no layout in a buffer: the backend chooses it")
            }
            _ => return Ok(()),
        };
        Err(invalid(problem))
    }

    fn check_attachments(&self, pass: &Pass) -> Result<(), GraphError> {
        let outputs = pass
            .outputs
            .iter()
            .filter(|output| matches!(output.attachment, Attachment::Color | Attachment::Depth))
            .map(|output| (output.resource, output.attachment == Attachment::Depth));
        let depth_tests = pass.inputs.iter().filter(|input| input.usage == ReadUsage::DepthTest);
        let attachments = outputs.chain(depth_tests.map(|input| (input.resource, true)));

        let mut depth: Option<&Resource> = None;
        let mut sized: Option<&Resource> = None;
        for (id, is_depth) in attachments {
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

            let Some(first) = sized else {
                sized = Some(resource);
                continue;
            };

            if first.size() != resource.size() {
                return Err(GraphError::AttachmentSizeMismatch {
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
