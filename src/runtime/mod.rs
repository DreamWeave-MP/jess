//! `ScriptDomain`s: isolated Luau authorities inside one process, over l3i.
//!
//! One process is not one VM. A [`ScriptDomain`] is the unit of script authority, module state,
//! lifetime, scheduling, diagnostics, and native-resource ownership; a process holds as many as
//! it needs, and they share nothing.
//!
//! - A domain owns one primary l3i `Runtime`, built by a [`RuntimeFactory`], and has an explicit
//!   lifecycle ([`DomainState`]), a stable [`DomainId`], and a [`Generation`].
//! - [`ContextSpec`] declares *when* code may run: the scheduler [`Phase`]s of a context.
//!   Script code binds handlers to them; the host enters them through
//!   [`ScriptDomain::run_phase`], [`ScriptDomain::advance`], and queued events.
//! - [`Grant`] and [`CapabilitySet`] say *what* code may do. The host holds [`RootAuthority`];
//!   a domain, a context, and a worker each hold a subset of whoever made them, and native
//!   services check it with [`require_capability`].
//! - Workers ([`WorkerSpec`]) are isolated Luau states that run beside the domain and exchange
//!   [`Message`]s with it: a small set of transferable values, never a shared table.
//! - [`Services`] are the native state a host attaches to a domain by type
//!   ([`DomainSpec::service`]); a binding finds them through [`service`], and a worker sees only
//!   the ones its spec shares.
//! - Everything the runtime reports carries an [`Attribution`]: [`Diagnostic`]s for failures,
//!   [`ProfileEvent`]s for cost.
//!
//! The module knows nothing about application-specific formats, sessions, workspaces, or GUI. It takes a
//! factory for runtimes and never names `jess::lua`; `jess::lua::plan` is one plan it can run.
//!
//! # Threads
//!
//! l3i's `Runtime` and `RuntimePlan` are not `Send`, and the module is built on that fact rather
//! than around it. A domain stays on the thread that created it:
//!
//! ```compile_fail
//! fn assert_send<T: Send>() {}
//! assert_send::<jess::runtime::ScriptDomain>();
//! ```
//!
//! because its runtime cannot move:
//!
//! ```compile_fail
//! fn assert_send<T: Send>() {}
//! assert_send::<l3i::Runtime>();
//! ```
//!
//! and neither can a plan, which is why workers take a factory and build their plan where they
//! run:
//!
//! ```compile_fail
//! fn assert_send<T: Send>() {}
//! assert_send::<std::rc::Rc<l3i::extension::RuntimePlan>>();
//! ```
//!
//! What does cross threads is `Send` by construction:
//!
//! ```
//! fn assert_send<T: Send>() {}
//! assert_send::<jess::runtime::Message>();
//! assert_send::<jess::runtime::DomainMailbox>();
//! assert_send::<jess::runtime::Diagnostic>();
//! assert_send::<jess::runtime::ProfileEvent>();
//! assert_send::<std::sync::Arc<dyn jess::runtime::RuntimeFactory>>();
//! ```

#![warn(missing_docs)]

mod capability;
pub mod chunk;
mod context;
mod diagnostics;
mod domain;
mod error;
mod ids;
mod log;
pub mod luau;
mod message;
mod profile;
mod resources;
mod service;
mod worker;

pub use capability::{
    Authority, CapabilityDenied, CapabilitySet, Grant, Grantor, RootAuthority, WORKER_SPAWN, current_authority,
    require_capability, service,
};
pub use context::{Budget, ContextInfo, ContextSpec, FixedStep, Phase};
pub use diagnostics::{
    Attribution, CollectedDiagnostics, Diagnostic, DiagnosticKind, DiagnosticSink, DiscardDiagnostics, Severity,
};
pub use domain::{
    DomainMailbox, DomainSpec, DomainState, EventLimits, EventReport, EventTarget, FromPlan, MailboxClosed, PhaseReport,
    PostRefused, QueueFull, RuntimeFactory, ScriptDomain, SetupHook, TeardownReport, TickReport,
};
pub use error::{RuntimeError, WorkerError, WorkerStage};
pub use ids::{CapabilityId, ContextId, DomainId, Generation, ResourceId, WorkerId};
pub use l3i;
pub use log::{CollectedLogs, LogLevel, LogRecord, LogSink};
pub use message::{MAX_DEPTH, Message, MessageKey, ResourceHandle, TransferError, push_message};
pub use profile::{CollectedProfile, Direction, DiscardProfile, DomainStats, ProfileEvent, ProfileSink};
pub use resources::OwnedResource;
pub use service::Services;
pub use worker::{WORKER_QUEUE, WorkerFailurePolicy, WorkerInfo, WorkerSpec, WorkerState};
