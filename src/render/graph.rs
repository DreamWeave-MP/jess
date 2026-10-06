//! Graph registration and the checks each registration runs.

use std::collections::HashMap;
use std::fmt;

use crate::render::error::GraphError;
use crate::render::pass::{Attachment, LoadOp, Pass, ReadUsage};
use crate::render::target::{TargetFormat, TargetSize};

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

/// A target in one [`RenderGraph`]. Only meaningful for the graph that returned it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ResourceId(pub(crate) usize);

impl ResourceId {
    /// The target's registration index.
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

/// Whether a target holds color or depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ResourceKind {
    /// A color target: written through color attachments, sampled as a color texture.
    RenderTarget,
    /// A depth target: written through the depth attachment, read for depth testing or sampled
    /// as a depth texture.
    DepthTarget,
}

impl ResourceKind {
    /// The kind's name, as errors print it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RenderTarget => "render target",
            Self::DepthTarget => "depth target",
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(self.name())
    }
}

/// Who owns a target's memory.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ResourceOrigin {
    /// The graph. The target exists only between its first and last use, holds nothing before
    /// its first write, and may share memory with other transient targets.
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

/// A registered target.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    /// The target's name, unique within its graph.
    pub name: String,
    /// Color or depth.
    pub kind: ResourceKind,
    /// Pixel format; a depth format exactly when `kind` is [`ResourceKind::DepthTarget`].
    pub format: TargetFormat,
    /// Size, fixed or relative to the viewport.
    pub size: TargetSize,
    /// Whether the graph or the host owns it.
    pub origin: ResourceOrigin,
}

/// Targets and the passes that use them, before ordering.
///
/// Registration checks everything one target or one pass can get wrong on its own;
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
        format: TargetFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_resource(
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
        format: TargetFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_resource(
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
        format: TargetFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_resource(
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
        format: TargetFormat,
        size: TargetSize,
    ) -> Result<ResourceId, GraphError> {
        self.add_resource(name.into(), ResourceKind::DepthTarget, format, size, ResourceOrigin::Imported)
    }

    fn add_resource(
        &mut self,
        name: String,
        kind: ResourceKind,
        format: TargetFormat,
        size: TargetSize,
        origin: ResourceOrigin,
    ) -> Result<ResourceId, GraphError> {
        if name.is_empty() {
            return Err(GraphError::EmptyResourceName);
        }

        if self.resource_ids.contains_key(&name) {
            return Err(GraphError::DuplicateResource { resource: name });
        }

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

        let id = ResourceId(self.resources.len());
        self.resource_ids.insert(name.clone(), id);
        self.resources.push(Resource {
            name,
            kind,
            format,
            size,
            origin,
        });
        Ok(id)
    }

    /// Registers a pass.
    ///
    /// # Errors
    ///
    /// When the pass gets something wrong on its own: an empty or taken name, an empty owner, no
    /// outputs, a target this graph never registered, a read or write its target's kind or format
    /// doesn't allow, a target used twice, or attachments that don't fit together (more than one
    /// depth attachment, or different sizes).
    pub fn add_pass(&mut self, pass: Pass) -> Result<PassId, GraphError> {
        self.check_pass(&pass)?;
        let id = PassId(self.passes.len());
        self.pass_ids.insert(pass.metadata.name.clone(), id);
        self.passes.push(pass);
        Ok(id)
    }

    /// Every registered target, indexed by [`ResourceId::index`].
    #[must_use]
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    /// Every registered pass, indexed by [`PassId::index`].
    #[must_use]
    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }

    /// The target `id` names, or `None` for an id from another graph.
    #[must_use]
    pub fn resource(&self, id: ResourceId) -> Option<&Resource> {
        self.resources.get(id.0)
    }

    /// The pass `id` names, or `None` for an id from another graph.
    #[must_use]
    pub fn pass(&self, id: PassId) -> Option<&Pass> {
        self.passes.get(id.0)
    }

    /// The target called `name`.
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

        // A pass touches a handful of targets, so the duplicate scans stay quadratic.
        for (index, input) in pass.inputs.iter().enumerate() {
            let resource = self.registered(name, input.resource)?;
            if input.usage.target_kind() != resource.kind {
                return Err(GraphError::IncompatibleRead {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    usage: input.usage,
                    kind: resource.kind,
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
            if output.attachment.target_kind() != resource.kind {
                return Err(GraphError::IncompatibleWrite {
                    pass: name.to_owned(),
                    resource: resource.name.clone(),
                    attachment: output.attachment,
                    kind: resource.kind,
                });
            }

            if let LoadOp::Clear(clear) = output.load {
                if !clear.fits(resource.format) {
                    return Err(GraphError::ClearValueMismatch {
                        pass: name.to_owned(),
                        resource: resource.name.clone(),
                        format: resource.format,
                        clear,
                    });
                }

                if !clear.is_valid() {
                    return Err(GraphError::InvalidClearValue {
                        pass: name.to_owned(),
                        resource: resource.name.clone(),
                        clear,
                    });
                }
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

        self.check_attachments(pass)
    }

    fn check_attachments(&self, pass: &Pass) -> Result<(), GraphError> {
        let outputs = pass
            .outputs
            .iter()
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

            if first.size != resource.size {
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
