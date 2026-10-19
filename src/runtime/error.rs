//! The runtime's errors, one variant per kind of failure, so a script error is never mistaken
//! for a capability denial, a lifecycle mistake, or a native fault.

use std::fmt;

use crate::runtime::capability::{CapabilityDenied, Grantor};
use crate::runtime::context::Phase;
use crate::runtime::diagnostics::Attribution;
use crate::runtime::domain::{DomainState, QueueFull};
use crate::runtime::ids::{CapabilityId, ContextId, DomainId, WorkerId};
use crate::runtime::message::TransferError;

/// Everything that can go wrong in the runtime.
#[derive(Clone, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum RuntimeError {
    /// Script code raised an error; attributed and never a panic.
    Script {
        /// Where it ran.
        attribution: Attribution,
        /// The phase it ran in, if a scheduler phase.
        phase: Option<Phase>,
        /// The Luau error message.
        message: String,
    },
    /// A capability check failed inside a dispatch.
    CapabilityDenied(CapabilityDenied),
    /// A capability check ran outside any domain's dispatch, where nothing holds authority.
    NoAuthority {
        /// The capability that was asked for.
        capability: CapabilityId,
    },
    /// A grantor tried to delegate capabilities it does not hold.
    OverDelegation {
        /// Who tried.
        grantor: Grantor,
        /// The capabilities it lacks.
        missing: Vec<CapabilityId>,
    },
    /// A capability name that is not dotted lowercase segments.
    InvalidCapabilityName(String),
    /// An action a domain's state does not allow.
    Lifecycle {
        /// The domain.
        domain: DomainId,
        /// Its state at the time.
        state: DomainState,
        /// What was asked.
        action: &'static str,
    },
    /// A scheduler entry from inside a running dispatch of the same domain.
    Reentrant {
        /// The domain.
        domain: DomainId,
        /// The phase that was already running.
        running: Phase,
        /// What was asked.
        action: &'static str,
    },
    /// No context of that name or id.
    UnknownContext {
        /// The domain.
        domain: DomainId,
        /// The name or id asked for.
        name: String,
    },
    /// A handler was bound to a phase the context's schedule does not include.
    UnscheduledPhase {
        /// The context.
        context: ContextId,
        /// The phase.
        phase: Phase,
    },
    /// A name that is not a scheduler phase.
    UnknownPhase(String),
    /// A worker asked to share a service its domain does not carry.
    UnknownService {
        /// The domain.
        domain: DomainId,
        /// The type name of the service asked for.
        service: &'static str,
    },
    /// No worker with that id in this domain.
    UnknownWorker {
        /// The domain.
        domain: DomainId,
        /// The id asked for.
        worker: WorkerId,
    },
    /// A value that cannot cross a boundary.
    Transfer(TransferError),
    /// A named event didn't fit in its domain's event queue.
    QueueFull(QueueFull),
    /// A worker failed.
    Worker(WorkerError),
    /// A worker did not answer within the time given.
    Timeout {
        /// The worker.
        worker: WorkerId,
    },
    /// l3i refused something at host level: a plan that does not finalize, a runtime that
    /// cannot be built, a value used across VMs. A host logic error, not a script's.
    Native(l3i::Error),
}

/// Where in its life a worker failed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkerStage {
    /// Its runtime could not be created.
    Spawn,
    /// Its module did not load, or did not return a handler function.
    Load,
    /// Its handler raised on a message.
    Handle,
}

impl fmt::Display for WorkerStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            WorkerStage::Spawn => "spawn",
            WorkerStage::Load => "load",
            WorkerStage::Handle => "handle",
        })
    }
}

/// A worker failure, attributed to its domain, generation, and worker.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkerError {
    /// Where.
    pub attribution: Attribution,
    /// The worker's chunk name, its source for diagnostics.
    pub module: String,
    /// When in its life.
    pub stage: WorkerStage,
    /// The error message.
    pub message: String,
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}) failed at {}: {}",
            self.attribution, self.module, self.stage, self.message
        )
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::Script {
                attribution,
                phase,
                message,
            } => {
                write!(f, "script error in {attribution}")?;
                if let Some(phase) = phase {
                    write!(f, " during {phase}")?;
                }
                write!(f, ": {message}")
            }
            RuntimeError::CapabilityDenied(denied) => denied.fmt(f),
            RuntimeError::NoAuthority { capability } => {
                write!(f, "capability '{}' checked outside any domain dispatch", capability.name())
            }
            RuntimeError::OverDelegation { grantor, missing } => {
                let names: Vec<&str> = missing.iter().map(|capability| capability.name()).collect();
                write!(
                    f,
                    "{grantor} cannot delegate capabilities it does not hold: {}",
                    names.join(", ")
                )
            }
            RuntimeError::InvalidCapabilityName(name) => {
                write!(f, "'{name}' is not a capability name (dotted lowercase segments)")
            }
            RuntimeError::Lifecycle { domain, state, action } => {
                write!(f, "cannot {action} {domain} while it is {state}")
            }
            RuntimeError::Reentrant { domain, running, action } => {
                write!(
                    f,
                    "cannot {action} {domain} from inside its own {running} dispatch; queue an event instead"
                )
            }
            RuntimeError::UnknownContext { domain, name } => write!(f, "{domain} has no context '{name}'"),
            RuntimeError::UnscheduledPhase { context, phase } => {
                write!(f, "{context} does not schedule the {phase} phase")
            }
            RuntimeError::UnknownPhase(name) => write!(f, "'{name}' is not a scheduler phase"),
            RuntimeError::UnknownService { domain, service } => {
                write!(f, "{domain} carries no service of type {service} for a worker to share")
            }
            RuntimeError::UnknownWorker { domain, worker } => write!(f, "{domain} has no {worker}"),
            RuntimeError::Transfer(error) => error.fmt(f),
            RuntimeError::QueueFull(full) => full.fmt(f),
            RuntimeError::Worker(error) => error.fmt(f),
            RuntimeError::Timeout { worker } => write!(f, "{worker} did not answer in time"),
            RuntimeError::Native(error) => write!(f, "l3i: {error}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<l3i::Error> for RuntimeError {
    fn from(error: l3i::Error) -> Self {
        RuntimeError::Native(error)
    }
}

impl From<TransferError> for RuntimeError {
    fn from(error: TransferError) -> Self {
        RuntimeError::Transfer(error)
    }
}

impl From<CapabilityDenied> for RuntimeError {
    fn from(denied: CapabilityDenied) -> Self {
        RuntimeError::CapabilityDenied(denied)
    }
}

impl From<RuntimeError> for l3i::Error {
    /// A runtime error as a bound function raises it: denials are permission errors, host
    /// mistakes stay logic errors, and everything else is the script-visible message.
    fn from(error: RuntimeError) -> Self {
        match error {
            RuntimeError::CapabilityDenied(_) | RuntimeError::NoAuthority { .. } | RuntimeError::OverDelegation { .. } => {
                l3i::Error::permission(error.to_string())
            }
            RuntimeError::Native(native) => native,
            other => l3i::Error::runtime(other.to_string()),
        }
    }
}

/// The message of an error a call from Rust into Luau raised, as the script wrote it: without
/// the `Lua error: ` l3i puts in front.
pub fn script_message(error: &l3i::Error) -> String {
    let message = error.to_string();
    message
        .strip_prefix("Lua error: ")
        .map_or_else(|| message.clone(), str::to_owned)
}
