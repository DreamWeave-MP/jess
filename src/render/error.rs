//! Why a graph was rejected.

use std::fmt;

use crate::render::graph::{ResourceId, ResourceKind};
use crate::render::pass::{Attachment, ClearValue, ReadUsage};
use crate::render::target::{TargetFormat, TargetSize};

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
        format: TargetFormat,
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
    /// A pass reads the same target twice, or writes it twice.
    DuplicateAccess {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
    },
    /// A pass reads and writes the same target.
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
        format: TargetFormat,
        /// The clear value.
        clear: ClearValue,
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
    /// A pass reads a transient target no pass writes.
    ReadWithoutWriter {
        /// The first pass that reads it.
        pass: String,
        /// The target.
        resource: String,
    },
    /// The first pass to write a transient target loads its contents, which don't exist yet.
    LoadWithoutContents {
        /// The pass.
        pass: String,
        /// The target.
        resource: String,
    },
    /// A pass reads or loads contents the previous writer discarded.
    ReadDiscarded {
        /// The pass that reads or loads.
        pass: String,
        /// The target.
        resource: String,
        /// The pass that discarded it.
        writer: String,
    },
    /// The passes' dependencies form a cycle.
    Cycle {
        /// The cycle in execution order, starting and ending with its earliest-registered pass.
        passes: Vec<String>,
    },
    /// No pass stores into an imported target, so every pass would be culled.
    NoImportedOutput {
        /// The graph.
        graph: String,
    },
}

impl fmt::Display for GraphError {
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
            Self::NoOutputs { pass } => write!(formatter, "pass \"{pass}\" writes nothing"),
            Self::UnknownResource { pass, resource } => {
                write!(
                    formatter,
                    "pass \"{pass}\" uses {resource}, which this graph never registered"
                )
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
                usage.target_kind()
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
                attachment.target_kind()
            ),
            Self::ClearValueMismatch {
                pass,
                resource,
                format,
                clear,
            } => {
                write!(formatter, "pass \"{pass}\" clears \"{resource}\" ({format}) with {clear:?}")
            }
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
            Self::UnusedResource { resource } => write!(formatter, "no pass uses target \"{resource}\""),
            Self::ReadWithoutWriter { pass, resource } => {
                write!(formatter, "pass \"{pass}\" reads \"{resource}\", which no pass writes")
            }
            Self::LoadWithoutContents { pass, resource } => {
                write!(
                    formatter,
                    "pass \"{pass}\" loads \"{resource}\" before any pass has written it"
                )
            }
            Self::ReadDiscarded { pass, resource, writer } => {
                write!(
                    formatter,
                    "pass \"{pass}\" reads \"{resource}\", which pass \"{writer}\" discards"
                )
            }
            Self::Cycle { passes } => write!(formatter, "dependency cycle: {}", passes.join(" -> ")),
            Self::NoImportedOutput { graph } => {
                write!(
                    formatter,
                    "graph \"{graph}\" stores nothing into an imported target, so every pass would be culled"
                )
            }
        }
    }
}

impl std::error::Error for GraphError {}
