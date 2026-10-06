//! Structured diagnostics: every failure the runtime sees, attributed to the domain,
//! generation, context, and worker it happened in. Logs are a presentation of these, never the
//! other way round.

use std::fmt;
use std::sync::{Mutex, PoisonError};

use crate::runtime::context::Phase;
use crate::runtime::ids::{CapabilityId, ContextId, DomainId, Generation, WorkerId};

/// Where something happened: the domain and its generation, and the context or worker when
/// there was one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Attribution {
    /// The domain.
    pub domain: DomainId,
    /// The domain's generation at the time.
    pub generation: Generation,
    /// The context whose handler was running, if any.
    pub context: Option<ContextId>,
    /// The worker that was running, if any.
    pub worker: Option<WorkerId>,
}

impl Attribution {
    /// An attribution to the domain itself.
    #[must_use]
    pub const fn domain(domain: DomainId, generation: Generation) -> Attribution {
        Attribution {
            domain,
            generation,
            context: None,
            worker: None,
        }
    }

    /// This attribution narrowed to a context.
    #[must_use]
    pub const fn in_context(self, context: ContextId) -> Attribution {
        Attribution {
            context: Some(context),
            ..self
        }
    }

    /// This attribution narrowed to a worker.
    #[must_use]
    pub const fn in_worker(self, worker: WorkerId) -> Attribution {
        Attribution {
            worker: Some(worker),
            ..self
        }
    }
}

impl fmt::Display for Attribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.domain, self.generation)?;
        if let Some(context) = self.context {
            write!(f, " {context}")?;
        }
        if let Some(worker) = self.worker {
            write!(f, " {worker}")?;
        }
        Ok(())
    }
}

/// How much a diagnostic matters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Severity {
    /// Bookkeeping worth seeing, such as stale work discarded on purpose.
    Info,
    /// Something a package author should look at.
    Warning,
    /// Script code failed or was refused.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// What kind of thing a diagnostic reports.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// A context handler raised a Luau error.
    HandlerFailed,
    /// A worker failed at spawn, load, or on a message, or was stopped by its failure policy.
    WorkerFailed,
    /// A native service refused a capability check.
    CapabilityDenied,
    /// Work queued for an earlier generation was dropped instead of reaching the new code.
    StaleGenerationDiscarded,
    /// A lifecycle action was asked of a domain in a state that does not allow it.
    InvalidLifecycleTransition,
    /// A value that cannot cross a worker or event boundary.
    TransferUnsupported,
    /// A worker could not be spawned.
    SpawnRefused,
    /// Work left in a queue at teardown was dropped.
    TeardownDiscarded,
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DiagnosticKind::HandlerFailed => "handler failed",
            DiagnosticKind::WorkerFailed => "worker failed",
            DiagnosticKind::CapabilityDenied => "capability denied",
            DiagnosticKind::StaleGenerationDiscarded => "stale generation event discarded",
            DiagnosticKind::InvalidLifecycleTransition => "invalid lifecycle transition",
            DiagnosticKind::TransferUnsupported => "transfer type unsupported",
            DiagnosticKind::SpawnRefused => "worker spawn refused",
            DiagnosticKind::TeardownDiscarded => "queued work discarded at teardown",
        })
    }
}

/// One structured diagnostic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// Where it happened.
    pub attribution: Attribution,
    /// How much it matters.
    pub severity: Severity,
    /// What kind of thing it is.
    pub kind: DiagnosticKind,
    /// The message, in the runtime's or the script's words.
    pub message: String,
    /// The scheduler phase that was running, if any.
    pub phase: Option<Phase>,
    /// The capability involved, for denials.
    pub capability: Option<CapabilityId>,
    /// `source:line` of the script code involved, when the runtime knows it.
    pub source: Option<String>,
}

impl Diagnostic {
    pub(crate) fn new(
        attribution: Attribution,
        severity: Severity,
        kind: DiagnosticKind,
        message: impl Into<String>,
    ) -> Self {
        Diagnostic {
            attribution,
            severity,
            kind,
            message: message.into(),
            phase: None,
            capability: None,
            source: None,
        }
    }

    pub(crate) fn phase(mut self, phase: Phase) -> Self {
        self.phase = Some(phase);
        self
    }

    pub(crate) fn capability(mut self, capability: CapabilityId) -> Self {
        self.capability = Some(capability);
        self
    }

    pub(crate) fn source(mut self, source: Option<String>) -> Self {
        self.source = source.filter(|source| !source.is_empty());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} ({}): {}",
            self.severity, self.attribution, self.kind, self.message
        )?;
        if let Some(phase) = self.phase {
            write!(f, " during {phase}")?;
        }
        if let Some(source) = &self.source {
            write!(f, " at {source}")?;
        }
        Ok(())
    }
}

/// Where a domain's diagnostics go. Workers report from their own threads, so a sink is shared
/// across them.
pub trait DiagnosticSink: Send + Sync + 'static {
    /// Receives one diagnostic.
    fn report(&self, diagnostic: Diagnostic);
}

/// A sink that keeps every diagnostic, for tests and for hosts that render them later.
#[derive(Debug, Default)]
pub struct CollectedDiagnostics {
    entries: Mutex<Vec<Diagnostic>>,
}

impl CollectedDiagnostics {
    /// An empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything reported so far, in order.
    #[must_use]
    pub fn entries(&self) -> Vec<Diagnostic> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Takes everything reported so far, leaving the collection empty.
    #[must_use]
    pub fn drain(&self) -> Vec<Diagnostic> {
        std::mem::take(&mut *self.entries.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The reported diagnostics of one kind.
    #[must_use]
    pub fn of_kind(&self, kind: DiagnosticKind) -> Vec<Diagnostic> {
        self.entries()
            .into_iter()
            .filter(|diagnostic| diagnostic.kind == kind)
            .collect()
    }
}

impl DiagnosticSink for CollectedDiagnostics {
    fn report(&self, diagnostic: Diagnostic) {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner).push(diagnostic);
    }
}

/// A sink that drops everything. For a host that truly wants no diagnostics; it has to say so.
#[derive(Debug, Default)]
pub struct DiscardDiagnostics;

impl DiagnosticSink for DiscardDiagnostics {
    fn report(&self, _: Diagnostic) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribution_and_diagnostics_display_every_dimension_they_have() {
        let domain = DomainId::allocate();
        let context = ContextId::allocate();
        let attribution = Attribution::domain(domain, Generation::FIRST).in_context(context);
        assert_eq!(attribution.to_string(), format!("{domain} gen#1 {context}"));
        let diagnostic = Diagnostic::new(attribution, Severity::Error, DiagnosticKind::HandlerFailed, "boom")
            .phase(Phase::Update)
            .source(Some("main.luau:3".to_owned()));
        assert_eq!(
            diagnostic.to_string(),
            format!("[error] {domain} gen#1 {context} (handler failed): boom during update at main.luau:3")
        );
        let collected = CollectedDiagnostics::new();
        collected.report(diagnostic.clone());
        assert_eq!(collected.of_kind(DiagnosticKind::HandlerFailed), vec![diagnostic]);
        assert_eq!(collected.of_kind(DiagnosticKind::WorkerFailed), []);
        assert_eq!(collected.drain().len(), 1);
        assert_eq!(collected.entries(), []);
    }
}
