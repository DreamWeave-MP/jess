//! A backend-independent render graph.
//!
//! A [`RenderGraph`] is a set of named targets and the passes that read and write them. Nothing
//! here draws anything or names a GPU API: the graph is the plan a backend executes.
//!
//! Building one registers targets ([`RenderGraph::add_render_target`],
//! [`RenderGraph::add_depth_target`], and the `import_*` pair for targets the host owns, such as
//! the window's backbuffer) and passes ([`RenderGraph::add_pass`]). Each pass is checked as it is
//! registered. [`RenderGraph::compile`] then checks the graph as a whole and produces a
//! [`CompiledGraph`]: the pass order, the passes nothing consumes, every target's lifetime in that
//! order, and which transient targets can share memory. Its `Display` is the debug dump.
//!
//! # Dependency rules
//!
//! Passes name the targets they use; the order comes from those uses, not from the order passes
//! were registered in.
//!
//! - **Read after write.** A read sees a target's final contents, so every pass that reads a
//!   target runs after the last pass that writes it.
//! - **Write after write.** The passes that write one target run in registration order. A write
//!   whose [`LoadOp`] is [`LoadOp::Load`] continues the previous writer's contents; any other load
//!   op replaces them.
//! - **Write after read** never becomes an edge. Every reader already follows every writer, so a
//!   graph that needs a target written after it is read has a cycle, and compiling it fails with
//!   the cycle's passes named. Use a second target: aliasing gives the memory back.
//!
//! Among passes the rules leave unordered, registration order breaks the tie, so the same graph
//! always compiles to the same order.
//!
//! # Culling
//!
//! Imported targets are what the graph produces. A pass survives compilation when the host can
//! observe its work: it is the last to store into an imported target, or a surviving pass reads
//! or loads what it stored. Every other pass is culled and listed as such.

#![warn(missing_docs)]

mod compile;
mod dump;
mod error;
mod graph;
mod pass;
mod target;

pub use compile::{AliasGroup, CompiledGraph, Dependency, DependencyKind, Lifetime};
pub use error::GraphError;
pub use graph::{PassId, RenderGraph, Resource, ResourceId, ResourceKind, ResourceOrigin};
pub use pass::{
    Attachment, ClearValue, LoadOp, Pass, PassInput, PassKind, PassMetadata, PassOutput, ReadUsage, SourceLocation, StoreOp,
};
pub use target::{Extent, SampleKind, TargetSize, TextureFormat};
