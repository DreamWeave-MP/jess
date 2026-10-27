//! Passes and the ways they touch targets and buffers.

use std::fmt;

use crate::render::graph::{ResourceId, ResourceKind};
use crate::render::target::TextureFormat;

/// What a pass does.
///
/// A backend picks its pipeline shape from this, and the kind limits what the pass may read and
/// write: [`PassKind::reads`] and [`PassKind::writes`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PassKind {
    /// Draws scene geometry.
    Mesh,
    /// Draws one triangle over its attachments, for post-processing and composition.
    Fullscreen,
    /// Runs a compute program, writing buffers.
    Compute,
    /// Copies one resource into another: a target into a buffer to read back, say.
    Copy,
}

impl PassKind {
    /// The kind's name, as errors and dumps print it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Fullscreen => "fullscreen",
            Self::Compute => "compute",
            Self::Copy => "copy",
        }
    }

    /// Whether a pass of this kind may read a resource as `usage`. Drawing passes read anything
    /// but a copy's source; compute passes read textures and buffers; a copy reads its source.
    #[must_use]
    pub const fn reads(self, usage: ReadUsage) -> bool {
        match self {
            Self::Mesh | Self::Fullscreen => !matches!(usage, ReadUsage::Copy),
            Self::Compute => matches!(usage, ReadUsage::Texture | ReadUsage::DepthTexture | ReadUsage::Buffer),
            Self::Copy => matches!(usage, ReadUsage::Copy),
        }
    }

    /// Whether a pass of this kind may write through `attachment`. Drawing passes write color and
    /// depth attachments; compute passes write storage; a copy writes its destination.
    #[must_use]
    pub const fn writes(self, attachment: Attachment) -> bool {
        match self {
            Self::Mesh | Self::Fullscreen => matches!(attachment, Attachment::Color | Attachment::Depth),
            Self::Compute => matches!(attachment, Attachment::Storage),
            Self::Copy => matches!(attachment, Attachment::Copy),
        }
    }
}

impl fmt::Display for PassKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.pad(self.name())
    }
}

/// Where a pass was declared, for profilers and error reports.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SourceLocation {
    /// The declaring file, as its author would recognize it.
    pub file: String,
    /// The 1-based line.
    pub line: u32,
    /// The 1-based column, or 0 where the declaring code knows only the line.
    pub column: u32,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.column == 0 {
            return write!(formatter, "{}:{}", self.file, self.line);
        }
        write!(formatter, "{}:{}:{}", self.file, self.line, self.column)
    }
}

/// Who a pass belongs to. Profilers and graph inspectors attribute work with this.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct PassMetadata {
    /// The pass's name, unique within its graph.
    pub name: String,
    /// The package or subsystem that declared the pass.
    pub owner: String,
    /// Where the pass was declared, when the declaring code knows.
    pub source: Option<SourceLocation>,
}

impl PassMetadata {
    /// Metadata with no source location.
    pub fn new(name: impl Into<String>, owner: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            owner: owner.into(),
            source: None,
        }
    }

    /// The same metadata, declared at `file:line:column`.
    #[must_use]
    pub fn with_source(self, file: impl Into<String>, line: u32, column: u32) -> Self {
        Self {
            source: Some(SourceLocation {
                file: file.into(),
                line,
                column,
            }),
            ..self
        }
    }
}

/// How a pass reads a resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ReadUsage {
    /// Sampled as a color texture. Render targets only.
    Texture,
    /// Sampled as a depth texture. Depth targets only.
    DepthTexture,
    /// Bound as the pass's read-only depth attachment for depth testing. Depth targets only.
    DepthTest,
    /// Read by its shaders as a storage buffer, or by its draws for vertices, instances, or indirect
    /// counts. Buffers only.
    Buffer,
    /// Copied from. Any resource; only a copy pass reads this way.
    Copy,
}

impl ReadUsage {
    /// The kind of resource this read is allowed on, or `None` for any.
    #[must_use]
    pub const fn target_kind(self) -> Option<ResourceKind> {
        match self {
            Self::Texture => Some(ResourceKind::RenderTarget),
            Self::DepthTexture | Self::DepthTest => Some(ResourceKind::DepthTarget),
            Self::Buffer => Some(ResourceKind::Buffer),
            Self::Copy => None,
        }
    }

    /// Whether this read is allowed on a resource of `kind`.
    #[must_use]
    pub fn allows(self, kind: ResourceKind) -> bool {
        self.target_kind().is_none_or(|allowed| allowed == kind)
    }

    /// Whether it reads one level of one layer, whether or not it names which: a depth test binds
    /// one as an attachment, and a copy copies one. The others read the whole target unless they
    /// name one.
    #[must_use]
    pub const fn reads_one(self) -> bool {
        matches!(self, Self::DepthTest | Self::Copy)
    }

    /// The usage's description, as errors print it.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Texture => "a color texture",
            Self::DepthTexture => "a depth texture",
            Self::DepthTest => "its depth-test attachment",
            Self::Buffer => "a storage buffer",
            Self::Copy => "a copy's source",
        }
    }
}

/// One mip level of one array layer of a target: what an attachment draws into, and what a read
/// or a copy can name instead of the whole target.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Subresource {
    /// The mip level, from 0, the full size.
    pub level: u32,
    /// The array layer, from 0.
    pub layer: u32,
}

impl Subresource {
    /// `level` of `layer`.
    #[must_use]
    pub const fn new(level: u32, layer: u32) -> Self {
        Self { level, layer }
    }
}

impl fmt::Display for Subresource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "level {} of layer {}", self.level, self.layer)
    }
}

/// A resource a pass reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PassInput {
    /// The resource read.
    pub resource: ResourceId,
    /// How it is read.
    pub usage: ReadUsage,
    /// The one level of one layer read, or `None` for the whole target (a buffer is always
    /// whole). A copy reads level 0 of layer 0 unless it names another.
    pub subresource: Option<Subresource>,
}

impl PassInput {
    /// The one level of one layer it reads, or `None` when it reads every one: a usage that
    /// [reads one](ReadUsage::reads_one) reads level 0 of layer 0 unless it names another.
    #[must_use]
    pub fn one(&self) -> Option<Subresource> {
        self.subresource
            .or_else(|| self.usage.reads_one().then_some(Subresource { level: 0, layer: 0 }))
    }
}

/// How a pass writes a resource: the attachment slot a target is bound to, a storage binding, or
/// a copy's destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Attachment {
    /// A color attachment. Render targets only.
    Color,
    /// The depth attachment. Depth targets only; a pass has at most one.
    Depth,
    /// A storage buffer its shaders write. Buffers only.
    Storage,
    /// A copy's destination. Any resource; only a copy pass writes this way.
    Copy,
}

impl Attachment {
    /// The kind of resource this attachment accepts, or `None` for any.
    #[must_use]
    pub const fn target_kind(self) -> Option<ResourceKind> {
        match self {
            Self::Color => Some(ResourceKind::RenderTarget),
            Self::Depth => Some(ResourceKind::DepthTarget),
            Self::Storage => Some(ResourceKind::Buffer),
            Self::Copy => None,
        }
    }

    /// Whether this attachment accepts a resource of `kind`.
    #[must_use]
    pub fn allows(self, kind: ResourceKind) -> bool {
        self.target_kind().is_none_or(|allowed| allowed == kind)
    }

    /// The attachment's name, as errors print it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Depth => "depth",
            Self::Storage => "storage",
            Self::Copy => "copy",
        }
    }
}

/// The value a [`LoadOp::Clear`] fills a resource with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClearValue {
    /// RGBA for float and normalized color formats. Components must be finite.
    Color([f32; 4]),
    /// RGBA for unsigned integer color formats.
    Uint([u32; 4]),
    /// RGBA for signed integer color formats.
    Sint([i32; 4]),
    /// Depth for depth formats, within `0.0..=1.0`.
    Depth(f32),
    /// Zeros: the only value a buffer clears to, and not one a target does.
    Zero,
}

impl ClearValue {
    /// Whether this value is the right kind for a target of `format`.
    #[must_use]
    pub const fn fits(self, format: TextureFormat) -> bool {
        match self {
            Self::Color(_) => !format.is_depth() && !format.is_uint() && !format.is_sint(),
            Self::Uint(_) => format.is_uint(),
            Self::Sint(_) => format.is_sint(),
            Self::Depth(_) => format.is_depth(),
            Self::Zero => false,
        }
    }

    /// Whether the value is in range: finite color components, depth within `0.0..=1.0`.
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::Color(color) => color.iter().all(|component| component.is_finite()),
            Self::Uint(_) | Self::Sint(_) | Self::Zero => true,
            Self::Depth(depth) => (0.0..=1.0).contains(&depth),
        }
    }
}

/// What a written resource holds when its pass starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoadOp {
    /// Filled with a value. Replaces the previous contents.
    Clear(ClearValue),
    /// The previous writer's contents. The pass depends on them, so something must have stored
    /// them: an earlier writer, or the host for an imported target.
    Load,
    /// Undefined. The pass promises to overwrite every pixel or byte it cares about.
    DontCare,
}

/// What happens to a written resource's contents when its pass ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StoreOp {
    /// Kept for later passes and, for imported targets, the host.
    Store,
    /// Thrown away. Nothing may read or load them afterwards.
    Discard,
}

/// A resource a pass writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassOutput {
    /// The resource written.
    pub resource: ResourceId,
    /// How it is written.
    pub attachment: Attachment,
    /// What it holds when the pass starts.
    pub load: LoadOp,
    /// What happens to it when the pass ends.
    pub store: StoreOp,
    /// The level of the layer of a target it writes; level 0 of layer 0 by default. A buffer is
    /// written whole.
    pub subresource: Subresource,
    /// A multisampled color attachment's resolve: the single-sample target, and its level and
    /// layer, that its samples average into at the end of the pass.
    pub resolve: Option<(ResourceId, Subresource)>,
}

impl PassOutput {
    /// The same write, into `level` of `layer`.
    #[must_use]
    pub const fn at(self, level: u32, layer: u32) -> Self {
        Self {
            subresource: Subresource { level, layer },
            ..self
        }
    }

    /// The same write, its samples resolved into `level` of `layer` of `target` when the pass
    /// ends.
    #[must_use]
    pub const fn resolving(self, target: ResourceId, level: u32, layer: u32) -> Self {
        Self {
            resolve: Some((target, Subresource { level, layer })),
            ..self
        }
    }

    /// A color attachment write.
    #[must_use]
    pub const fn color(resource: ResourceId, load: LoadOp, store: StoreOp) -> Self {
        Self {
            resource,
            attachment: Attachment::Color,
            load,
            store,
            subresource: Subresource { level: 0, layer: 0 },
            resolve: None,
        }
    }

    /// A depth attachment write.
    #[must_use]
    pub const fn depth(resource: ResourceId, load: LoadOp, store: StoreOp) -> Self {
        Self {
            resource,
            attachment: Attachment::Depth,
            load,
            store,
            subresource: Subresource { level: 0, layer: 0 },
            resolve: None,
        }
    }

    /// A storage buffer write, by a compute pass.
    #[must_use]
    pub const fn storage(resource: ResourceId, load: LoadOp, store: StoreOp) -> Self {
        Self {
            resource,
            attachment: Attachment::Storage,
            load,
            store,
            subresource: Subresource { level: 0, layer: 0 },
            resolve: None,
        }
    }

    /// A copy's destination.
    #[must_use]
    pub const fn copy(resource: ResourceId, load: LoadOp, store: StoreOp) -> Self {
        Self {
            resource,
            attachment: Attachment::Copy,
            load,
            store,
            subresource: Subresource { level: 0, layer: 0 },
            resolve: None,
        }
    }
}

/// Where a copy starts in a resource: a texel of a target's subresource, or a byte of a buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CopyAt {
    /// The texel `x` across and `y` down.
    Texel {
        /// Across.
        x: u32,
        /// Down.
        y: u32,
    },
    /// A byte offset, a multiple of 4.
    Byte(u64),
}

/// How much a copy copies.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum CopySize {
    /// A rectangle of texels.
    Texels {
        /// Across.
        width: u32,
        /// Down.
        height: u32,
    },
    /// Bytes, a multiple of 4, between buffers.
    Bytes(u64),
}

/// What part of its source a copy pass copies, and where into its destination. Everything left
/// out is the start, and the whole of what the source has from there.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct CopyRegion {
    /// Where in the source it starts.
    pub from: Option<CopyAt>,
    /// Where in the destination it lands.
    pub to: Option<CopyAt>,
    /// How much it copies.
    pub size: Option<CopySize>,
}

impl CopyRegion {
    /// Whether it is the whole copy: everything from the start.
    #[must_use]
    pub const fn is_whole(&self) -> bool {
        self.from.is_none() && self.to.is_none() && self.size.is_none()
    }
}

/// One step of a render graph: what it does, what it reads, and what it writes.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    /// Name, owner, and declaration site.
    pub metadata: PassMetadata,
    /// What the pass does.
    pub kind: PassKind,
    /// Resources read, in declaration order.
    pub inputs: Vec<PassInput>,
    /// Resources written, in declaration order. A pass writes at least one.
    pub outputs: Vec<PassOutput>,
    /// A copy pass's region; only a copy has one other than the whole.
    pub region: CopyRegion,
}

impl Pass {
    /// A pass that reads and writes nothing yet.
    #[must_use]
    pub const fn new(kind: PassKind, metadata: PassMetadata) -> Self {
        Self {
            metadata,
            kind,
            inputs: Vec::new(),
            outputs: Vec::new(),
            region: CopyRegion {
                from: None,
                to: None,
                size: None,
            },
        }
    }

    /// The same pass, also reading `resource` as `usage`.
    #[must_use]
    pub fn read(mut self, resource: ResourceId, usage: ReadUsage) -> Self {
        self.inputs.push(PassInput {
            resource,
            usage,
            subresource: None,
        });
        self
    }

    /// The same pass, also reading `level` of `layer` of `resource` as `usage`.
    #[must_use]
    pub fn read_at(mut self, resource: ResourceId, usage: ReadUsage, level: u32, layer: u32) -> Self {
        self.inputs.push(PassInput {
            resource,
            usage,
            subresource: Some(Subresource { level, layer }),
        });
        self
    }

    /// The same copy pass, copying `region`.
    #[must_use]
    pub fn region(mut self, region: CopyRegion) -> Self {
        self.region = region;
        self
    }

    /// The same pass, also writing `output`.
    #[must_use]
    pub fn write(mut self, output: PassOutput) -> Self {
        self.outputs.push(output);
        self
    }

    /// The pass's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    /// The level and layer of each resource it writes: each output's, then each resolve's.
    pub fn writes(&self) -> impl Iterator<Item = (ResourceId, Subresource)> + '_ {
        let outputs = self.outputs.iter().map(|output| (output.resource, output.subresource));
        outputs.chain(self.outputs.iter().filter_map(|output| output.resolve))
    }

    /// Every resource it reads or writes, resolves included, in declaration order; a resource
    /// used several ways appears once for each.
    pub fn resources(&self) -> impl Iterator<Item = ResourceId> + '_ {
        let inputs = self.inputs.iter().map(|input| input.resource);
        inputs.chain(self.writes().map(|(resource, _)| resource))
    }
}
