//! The application's log stream: what scripts log, attributed to the domain, generation,
//! context, and worker that logged it, in one sink for the whole domain.
//!
//! A domain without a sink has no stream, and a logging module keeps its runtime's own
//! destination. With one ([`DomainSpec::logs`](crate::runtime::DomainSpec::logs)), every record its code
//! logs goes there, its workers' included, whether or not they share any service: a worker's
//! logs are the application's, like its diagnostics.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, PoisonError};

use crate::runtime::diagnostics::Attribution;
use crate::runtime::message::Message;

/// How much a log record matters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum LogLevel {
    /// Detail for whoever is debugging.
    Debug,
    /// Something worth knowing.
    Info,
    /// Something worth looking at.
    Warn,
    /// Something went wrong.
    Error,
}

impl LogLevel {
    /// The level named `name`: `debug`, `info`, `warn`, or `error`.
    #[must_use]
    pub fn parse(name: &str) -> Option<LogLevel> {
        match name {
            "debug" => Some(LogLevel::Debug),
            "info" => Some(LogLevel::Info),
            "warn" => Some(LogLevel::Warn),
            "error" => Some(LogLevel::Error),
            _ => None,
        }
    }

    /// The level's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        }
    }
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// One record in a domain's log stream.
#[derive(Clone, PartialEq, Debug)]
pub struct LogRecord {
    /// Who logged it: the runtime's attribution of the running code, never the script's word.
    pub attribution: Attribution,
    /// How much it matters.
    pub level: LogLevel,
    /// The logger's target.
    pub target: String,
    /// The message.
    pub message: String,
    /// Structured fields: text, finite numbers, and booleans.
    pub fields: BTreeMap<String, Message>,
    /// The chunk that logged, as the VM attributes it.
    pub source: Option<String>,
}

impl fmt::Display for LogRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {} {}: {}", self.level, self.attribution, self.target, self.message)?;
        if let Some(source) = &self.source {
            write!(f, " at {source}")?;
        }
        Ok(())
    }
}

/// Where a domain's log stream goes. Workers log from their own threads, so a sink is shared
/// across them.
pub trait LogSink: Send + Sync + 'static {
    /// Receives one record.
    fn record(&self, record: LogRecord);
}

/// A sink that keeps every record, for tests and for hosts that render them later.
#[derive(Debug, Default)]
pub struct CollectedLogs {
    entries: Mutex<Vec<LogRecord>>,
}

impl CollectedLogs {
    /// An empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything logged so far, in order.
    #[must_use]
    pub fn entries(&self) -> Vec<LogRecord> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Takes everything logged so far, leaving the collection empty.
    #[must_use]
    pub fn drain(&self) -> Vec<LogRecord> {
        std::mem::take(&mut *self.entries.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl LogSink for CollectedLogs {
    fn record(&self, record: LogRecord) {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner).push(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ids::{DomainId, Generation, WorkerId};

    #[test]
    fn a_record_displays_its_level_attribution_target_and_source() {
        let domain = DomainId::allocate();
        let worker = WorkerId::allocate();
        let record = LogRecord {
            attribution: Attribution::domain(domain, Generation::FIRST).in_worker(worker),
            level: LogLevel::parse("warn").unwrap(),
            target: "indexer".to_owned(),
            message: "slow".to_owned(),
            fields: BTreeMap::new(),
            source: Some("@app/worker.luau:4".to_owned()),
        };
        assert_eq!(
            record.to_string(),
            format!("[warn] {domain} gen#1 {worker} indexer: slow at @app/worker.luau:4")
        );
        assert_eq!(LogLevel::parse("trace"), None);
        let collected = CollectedLogs::new();
        collected.record(record.clone());
        assert_eq!(collected.drain(), vec![record]);
        assert_eq!(collected.entries(), []);
    }
}
