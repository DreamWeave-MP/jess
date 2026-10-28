//! Why a graph was rejected.

use std::fmt;

use crate::render::graph::{ResourceId, ResourceKind};
use crate::render::pass::{Attachment, ClearValue, PassKind, ReadUsage, Subresource};
use crate::render::target::{TargetSize, TextureFormat};

/// Why registering a target or pass, or compiling a graph, failed.
///
/// Passes and targets are named, not numbered, so the message points at the declaration to fix.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphError {
    /// A target was registered with an empty name.
    EmptyResourceName,
    /// A pass was registered with an empty name.
    EmptyPassName,
    /// A pass was registered with an empty owner.
    EmptyOwner {
        /// The pass.
        pass: String,
    },
    /// Two targets share a name.
    DuplicateResource {
        /// The name.
        resource: String,
    },
    /// Two passes share a name.
    DuplicatePass {
        /// The name.
        pass: String,
    },
    /// A render target got a depth format, or a depth target a color format.
    FormatKind {
        /// The target.
        resource: String,
        /// The format it was given.
        format: TextureFormat,
        /// The kind it was registered as.
        kind: ResourceKind,
    },
    /// A target's size is zero, negative, or not finite.
    InvalidSize {
        /// The target.
        resource: String,
        /// The size it was given.
        size: TargetSize,
    },
    /// A target's levels, layers, or samples don't fit together or its size.
    InvalidTargetShape {
        /// The target.
        resource: String,
        /// What it would need to be.
        problem: String,
    },
    /// A buffer sized by the viewport whose format or scale can't size one.
    InvalidBufferShape {
        /// The buffer.
        resource: String,
        /// What it would need.
        problem: String,
    },
    /// A buffer's size is zero or not a multiple of 4.
    InvalidBufferSize {
        /// The buffer.
        resource: String,
        /// The size it was given, in bytes.
        bytes: u64,
    },
    /// A pass writes nothing, so it can have no effect.
    NoOutputs {
        /// The pass.
        pass: String,
    },
    /// A pass names a target this graph never registered.
    UnknownResource {
        /// The pass.
        pass: String,
        /// The id it used.
        resource: ResourceId,
    },
    /// A pass names a level or layer its target doesn't have, or one of a buffer.
    SubresourceOutOfRange {
        /// The pass.
        pass: String,
        /// The resource.
        resource: String,
        /// What it named.
        subresource: Subresource,
    },
    /// A pass samples, copies, or stores into a multisampled target, which is only drawn into and
    /// resolved.
    Multisampled {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
        /// How the pass uses it.
        usage: &'static str,
    },
    /// A pass resolves a color attachment into a target it can't.
    InvalidResolve {
        /// The pass.
        pass: String,
        /// The attachment resolved.
        resource: String,
        /// What is wrong.
        problem: String,
    },
    /// A pass binds attachments of different sample counts.
    SampleCountMismatch {
        /// The pass.
        pass: String,
        /// The first attachment.
        first: String,
        /// The first attachment whose sample count differs from it.
        second: String,
    },
    /// A pass other than a copy names a copy region.
    RegionOutsideCopy {
        /// The pass.
        pass: String,
    },
    /// A pass reads the same level of a layer of a target twice, or writes it twice.
    DuplicateAccess {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
    },
    /// A pass reads and writes the same level of a layer of a target.
    ReadWriteConflict {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
    },
    /// A pass reads a target in a way its kind doesn't allow.
    IncompatibleRead {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
        /// How the pass reads it.
        usage: ReadUsage,
        /// What the target is.
        kind: ResourceKind,
    },
    /// A pass writes a target through an attachment its kind doesn't allow.
    IncompatibleWrite {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
        /// The attachment it is written through.
        attachment: Attachment,
        /// What the target is.
        kind: ResourceKind,
    },
    /// A pass clears a target with a value of the wrong kind for its format.
    ClearValueMismatch {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
        /// The target's format.
        format: TextureFormat,
        /// The clear value.
        clear: ClearValue,
    },
    /// A pass clears a buffer with a value other than zero.
    BufferClearValue {
        /// The pass.
        pass: String,
        /// The buffer.
        resource: String,
        /// The clear value.
        clear: ClearValue,
    },
    /// A pass reads a resource in a way its kind of pass can't: a compute pass reading a depth
    /// test, say.
    PassCantRead {
        /// The pass.
        pass: String,
        /// What kind of pass it is.
        kind: PassKind,
        /// How it reads.
        usage: ReadUsage,
    },
    /// A pass writes a resource in a way its kind of pass can't: a compute pass writing a color
    /// attachment, say.
    PassCantWrite {
        /// The pass.
        pass: String,
        /// What kind of pass it is.
        kind: PassKind,
        /// How it writes.
        attachment: Attachment,
    },
    /// A copy pass that doesn't copy one resource into one it fits.
    InvalidCopy {
        /// The pass.
        pass: String,
        /// What is wrong.
        problem: String,
    },
    /// A pass clears a target with a value out of range.
    InvalidClearValue {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
        /// The clear value.
        clear: ClearValue,
    },
    /// A pass binds two depth attachments.
    MultipleDepthAttachments {
        /// The pass.
        pass: String,
        /// The first depth attachment.
        first: String,
        /// The second.
        second: String,
    },
    /// A pass binds attachments of different sizes.
    AttachmentSizeMismatch {
        /// The pass.
        pass: String,
        /// The first attachment.
        first: String,
        /// The first attachment whose size differs from it.
        second: String,
    },
    /// A transient target no pass uses.
    UnusedResource {
        /// The target.
        resource: String,
    },
    /// A pass reads a transient target, or a level of a layer of one, no pass writes.
    ReadWithoutWriter {
        /// The first pass that reads it.
        pass: String,
        /// The target.
        resource: String,
        /// The level of the layer, for a target with more than one.
        subresource: Option<Subresource>,
    },
    /// The first pass to write a transient target loads its contents, which don't exist yet.
    LoadWithoutContents {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
        /// The level of the layer, for a target with more than one.
        subresource: Option<Subresource>,
    },
    /// A pass reads or loads contents the previous writer discarded.
    ReadDiscarded {
        /// The pass that reads or loads.
        pass: String,
        /// The target.
        resource: String,
        /// The level of the layer, for a target with more than one.
        subresource: Option<Subresource>,
        /// The pass that discarded it.
        writer: String,
    },
    /// The passes' dependencies form a cycle.
    Cycle {
        /// The cycle in execution order, starting and ending with its earliest-registered pass.
        passes: Vec<String>,
    },
    /// No pass stores into an imported resource, so every pass would be culled.
    NoImportedOutput {
        /// The graph.
        graph: String,
    },
}

impl fmt::Display for GraphError {
    #[allow(clippy::too_many_lines)] // one arm per error
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyResourceName => formatter.write_str("a target was registered with an empty name"),
            Self::EmptyPassName => formatter.write_str("a pass was registered with an empty name"),
            Self::EmptyOwner { pass } => write!(formatter, "pass \"{pass}\" has an empty owner"),
            Self::DuplicateResource { resource } => write!(formatter, "target \"{resource}\" is registered twice"),
            Self::DuplicatePass { pass } => write!(formatter, "pass \"{pass}\" is registered twice"),
            Self::FormatKind { resource, format, kind } => {
                write!(formatter, "{kind} \"{resource}\" can't have format {format}")
            }
            Self::InvalidSize { resource, size } => write!(formatter, "target \"{resource}\" has invalid size {size}"),
            Self::InvalidTargetShape { resource, problem } => {
                write!(formatter, "target \"{resource}\" needs {problem}")
            }
            Self::InvalidBufferShape { resource, problem } => {
                write!(formatter, "buffer \"{resource}\" needs {problem}")
            }
            Self::InvalidBufferSize { resource, bytes } => write!(
                formatter,
                "buffer \"{resource}\" has invalid size {bytes}: expected a nonzero multiple of 4 bytes"
            ),
            Self::NoOutputs { pass } => write!(formatter, "pass \"{pass}\" writes nothing"),
            Self::UnknownResource { pass, resource } => {
                write!(
                    formatter,
                    "pass \"{pass}\" uses {resource}, which this graph never registered"
                )
            }
            Self::SubresourceOutOfRange {
                pass,
                resource,
                subresource,
            } => {
                write!(
                    formatter,
                    "pass \"{pass}\" uses {subresource} of \"{resource}\", which it doesn't have"
                )
            }
            Self::Multisampled { pass, resource, usage } => write!(
                formatter,
                "pass \"{pass}\" uses multisampled \"{resource}\" as {usage}: a multisampled target is drawn into and resolved"
            ),
            Self::InvalidResolve { pass, resource, problem } => {
                write!(formatter, "pass \"{pass}\" can't resolve \"{resource}\": {problem}")
            }
            Self::SampleCountMismatch { pass, first, second } => write!(
                formatter,
                "pass \"{pass}\" binds attachments \"{first}\" and \"{second}\" of different sample counts"
            ),
            Self::RegionOutsideCopy { pass } => {
                write!(formatter, "pass \"{pass}\" names a copy region, and only a copy pass copies")
            }
            Self::DuplicateAccess { pass, resource } => {
                write!(formatter, "pass \"{pass}\" uses \"{resource}\" twice in the same direction")
            }
            Self::ReadWriteConflict { pass, resource } => {
                write!(formatter, "pass \"{pass}\" both reads and writes \"{resource}\"")
            }
            Self::IncompatibleRead {
                pass,
                resource,
                usage,
                kind,
            } => write!(
                formatter,
                "pass \"{pass}\" reads {kind} \"{resource}\" as {}, which only a {} allows",
                usage.description(),
                usage.target_kind().map_or("", ResourceKind::name)
            ),
            Self::IncompatibleWrite {
                pass,
                resource,
                attachment,
                kind,
            } => write!(
                formatter,
                "pass \"{pass}\" writes {kind} \"{resource}\" through a {} attachment, which only a {} allows",
                attachment.name(),
                attachment.target_kind().map_or("", ResourceKind::name)
            ),
            Self::ClearValueMismatch {
                pass,
                resource,
                format,
                clear,
            } => {
                write!(formatter, "pass \"{pass}\" clears \"{resource}\" ({format}) with {clear:?}")
            }
            Self::BufferClearValue { pass, resource, clear } => {
                write!(
                    formatter,
                    "pass \"{pass}\" clears buffer \"{resource}\" with {clear:?}: a buffer clears to Zero"
                )
            }
            Self::PassCantRead { pass, kind, usage } => {
                write!(
                    formatter,
                    "{kind} pass \"{pass}\" can't read anything as {}",
                    usage.description()
                )
            }
            Self::PassCantWrite { pass, kind, attachment } => {
                write!(
                    formatter,
                    "{kind} pass \"{pass}\" can't write through a {} attachment",
                    attachment.name()
                )
            }
            Self::InvalidCopy { pass, problem } => write!(formatter, "copy pass \"{pass}\" is invalid: {problem}"),
            Self::InvalidClearValue { pass, resource, clear } => {
                write!(formatter, "pass \"{pass}\" clears \"{resource}\" with out-of-range {clear:?}")
            }
            Self::MultipleDepthAttachments { pass, first, second } => {
                write!(
                    formatter,
                    "pass \"{pass}\" binds two depth attachments, \"{first}\" and \"{second}\""
                )
            }
            Self::AttachmentSizeMismatch { pass, first, second } => {
                write!(
                    formatter,
                    "pass \"{pass}\" binds attachments \"{first}\" and \"{second}\" of different sizes"
                )
            }
            Self::UnusedResource { resource } => write!(formatter, "no pass uses \"{resource}\""),
            Self::ReadWithoutWriter {
                pass,
                resource,
                subresource,
            } => {
                let resource = Named(resource, *subresource);
                write!(formatter, "pass \"{pass}\" reads {resource}, which no pass writes")
            }
            Self::LoadWithoutContents {
                pass,
                resource,
                subresource,
            } => {
                let resource = Named(resource, *subresource);
                write!(formatter, "pass \"{pass}\" loads {resource} before any pass has written it")
            }
            Self::ReadDiscarded {
                pass,
                resource,
                subresource,
                writer,
            } => {
                let resource = Named(resource, *subresource);
                write!(
                    formatter,
                    "pass \"{pass}\" reads {resource}, which pass \"{writer}\" discards"
                )
            }
            Self::Cycle { passes } => write!(formatter, "dependency cycle: {}", passes.join(" -> ")),
            Self::NoImportedOutput { graph } => {
                write!(
                    formatter,
                    "graph \"{graph}\" stores nothing into an imported resource, so every pass would be culled"
                )
            }
        }
    }
}

impl std::error::Error for GraphError {}

/// A resource's quoted name, and the level of the layer meant where there is one.
struct Named<'a>(&'a str, Option<Subresource>);

impl fmt::Display for Named<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.1 {
            Some(subresource) => write!(formatter, "{subresource} of \"{}\"", self.0),
            None => write!(formatter, "\"{}\"", self.0),
        }
    }
}
