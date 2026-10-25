//! Passes and the ways they touch targets.

use std::fmt;

use crate::render::graph::{ResourceId, ResourceKind};
use crate::render::target::TextureFormat;

/// What a pass draws.
///
/// A backend picks its pipeline shape from this. Compute passes are not modeled yet; the enum is
/// non-exhaustive so adding them breaks no one.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum PassKind {
    /// Draws scene geometry.
    Mesh,
    /// Draws one triangle over its attachments, for post-processing and composition.
    Fullscreen,
}

impl PassKind {
    /// The kind's name, as errors and dumps print it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Fullscreen => "fullscreen",
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
    /// The 1-based column.
    pub column: u32,
}

impl fmt::Display for SourceLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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

/// How a pass reads a target.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ReadUsage {
    /// Sampled as a color texture. Render targets only.
    Texture,
    /// Sampled as a depth texture. Depth targets only.
    DepthTexture,
    /// Bound as the pass's read-only depth attachment for depth testing. Depth targets only.
    DepthTest,
}

impl ReadUsage {
    /// The kind of target this read is allowed on.
    #[must_use]
    pub const fn target_kind(self) -> ResourceKind {
        match self {
            Self::Texture => ResourceKind::RenderTarget,
            Self::DepthTexture | Self::DepthTest => ResourceKind::DepthTarget,
        }
    }

    /// The usage's description, as errors print it.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::Texture => "a color texture",
            Self::DepthTexture => "a depth texture",
            Self::DepthTest => "its depth-test attachment",
        }
    }
}

/// A target a pass reads.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct PassInput {
    /// The target read.
    pub resource: ResourceId,
    /// How it is read.
    pub usage: ReadUsage,
}

/// Which attachment slot a pass writes a target through.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Attachment {
    /// A color attachment. Render targets only.
    Color,
    /// The depth attachment. Depth targets only; a pass has at most one.
    Depth,
}

impl Attachment {
    /// The kind of target this attachment accepts.
    #[must_use]
    pub const fn target_kind(self) -> ResourceKind {
        match self {
            Self::Color => ResourceKind::RenderTarget,
            Self::Depth => ResourceKind::DepthTarget,
        }
    }

    /// The attachment's name, as errors print it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Depth => "depth",
        }
    }
}

/// The value a [`LoadOp::Clear`] fills a target with.
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
}

impl ClearValue {
    /// Whether this value is the right kind for `format`.
    #[must_use]
    pub const fn fits(self, format: TextureFormat) -> bool {
        match self {
            Self::Color(_) => !format.is_depth() && !format.is_uint() && !format.is_sint(),
            Self::Uint(_) => format.is_uint(),
            Self::Sint(_) => format.is_sint(),
            Self::Depth(_) => format.is_depth(),
        }
    }

    /// Whether the value is in range: finite color components, depth within `0.0..=1.0`.
    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::Color(color) => color.iter().all(|component| component.is_finite()),
            Self::Uint(_) | Self::Sint(_) => true,
            Self::Depth(depth) => (0.0..=1.0).contains(&depth),
        }
    }
}

/// What a written target holds when its pass starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoadOp {
    /// Filled with a value. Replaces the previous contents.
    Clear(ClearValue),
    /// The previous writer's contents. The pass depends on them, so something must have stored
    /// them: an earlier writer, or the host for an imported target.
    Load,
    /// Undefined. The pass promises to overwrite every pixel it cares about.
    DontCare,
}

/// What happens to a written target's contents when its pass ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum StoreOp {
    /// Kept for later passes and, for imported targets, the host.
    Store,
    /// Thrown away. Nothing may read or load them afterwards.
    Discard,
}

/// A target a pass writes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassOutput {
    /// The target written.
    pub resource: ResourceId,
    /// The attachment slot it is bound to.
    pub attachment: Attachment,
    /// What it holds when the pass starts.
    pub load: LoadOp,
    /// What happens to it when the pass ends.
    pub store: StoreOp,
}

impl PassOutput {
    /// A color attachment write.
    #[must_use]
    pub const fn color(resource: ResourceId, load: LoadOp, store: StoreOp) -> Self {
        Self {
            resource,
            attachment: Attachment::Color,
            load,
            store,
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
        }
    }
}

/// One step of a render graph: what it draws, what it reads, and what it writes.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    /// Name, owner, and declaration site.
    pub metadata: PassMetadata,
    /// What the pass draws.
    pub kind: PassKind,
    /// Targets read, in declaration order.
    pub inputs: Vec<PassInput>,
    /// Targets written, in declaration order. A pass writes at least one.
    pub outputs: Vec<PassOutput>,
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
        }
    }

    /// The same pass, also reading `resource` as `usage`.
    #[must_use]
    pub fn read(mut self, resource: ResourceId, usage: ReadUsage) -> Self {
        self.inputs.push(PassInput { resource, usage });
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
}
