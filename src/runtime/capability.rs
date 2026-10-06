//! Capabilities: what code may do, as the host grants it.
//!
//! A capability is a dotted lowercase name (`input.observe`, `test.echo`) interned to a
//! [`CapabilityId`]. The host holds [`RootAuthority`] and grants a [`CapabilitySet`] to a domain;
//! a domain delegates subsets to its contexts, its workers, and its child domains, and can never
//! delegate what it does not hold ([`Grant::delegate`]).
//!
//! Native services check authority through the [`Authority`] the runtime installs on the thread
//! while script code runs: [`current_authority`] and [`require_capability`]. That ticket is set
//! around every scheduler dispatch and for a worker thread's life, so the check is right under
//! several domains and workers, and nothing about it is process-global mutable state.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use crate::runtime::diagnostics::{Attribution, Diagnostic, DiagnosticKind, DiagnosticSink, Severity};
use crate::runtime::error::RuntimeError;
use crate::runtime::ids::{CapabilityId, DomainId};
use crate::runtime::log::LogSink;
use crate::runtime::profile::{Counters, ProfileEvent, ProfileSink};
use crate::runtime::service::Services;

/// The capability a script needs to spawn a worker from Luau.
pub const WORKER_SPAWN: &str = "worker.spawn";

static NAMES: RwLock<Vec<&'static str>> = RwLock::new(Vec::new());

impl CapabilityId {
    /// The id of `name`, interning it on first use. A name is one or more segments of lowercase
    /// ASCII letters, digits, and hyphens, separated by dots.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::InvalidCapabilityName`] for any other spelling.
    ///
    /// # Panics
    ///
    /// If the process has interned `u32::MAX` distinct names, which no vocabulary reaches.
    pub fn intern(name: &str) -> Result<CapabilityId, RuntimeError> {
        if !is_valid_name(name) {
            return Err(RuntimeError::InvalidCapabilityName(name.to_owned()));
        }
        if let Some(id) = Self::lookup(name) {
            return Ok(id);
        }
        let mut names = NAMES.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = names.iter().position(|known| *known == name) {
            return Ok(Self::at(index));
        }
        // Interned once per distinct name for the life of the process, so `name()` can hand out
        // a `&'static str` without a lock.
        names.push(Box::leak(name.to_owned().into_boxed_str()));
        Ok(Self::at(names.len() - 1))
    }

    fn at(index: usize) -> CapabilityId {
        CapabilityId(u32::try_from(index).expect("fewer than u32::MAX capability names"))
    }

    /// The id of `name` if it was interned already.
    #[must_use]
    pub fn lookup(name: &str) -> Option<CapabilityId> {
        let names = NAMES.read().unwrap_or_else(PoisonError::into_inner);
        names.iter().position(|known| *known == name).map(Self::at)
    }

    /// The interned name.
    ///
    /// # Panics
    ///
    /// Never for an id [`CapabilityId::intern`] made, which is the only way to make one.
    #[must_use]
    pub fn name(self) -> &'static str {
        let names = NAMES.read().unwrap_or_else(PoisonError::into_inner);
        names.get(self.0 as usize).copied().expect("a CapabilityId comes from intern")
    }
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// A set of capabilities. Cheap to compare and to test for containment.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct CapabilitySet(BTreeSet<CapabilityId>);

impl CapabilitySet {
    /// The empty set.
    #[must_use]
    pub const fn empty() -> CapabilitySet {
        CapabilitySet(BTreeSet::new())
    }

    /// A set from capability names, interning them.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::InvalidCapabilityName`] for a name that is not one.
    pub fn from_names<S: AsRef<str>>(names: impl IntoIterator<Item = S>) -> Result<CapabilitySet, RuntimeError> {
        let mut set = BTreeSet::new();
        for name in names {
            set.insert(CapabilityId::intern(name.as_ref())?);
        }
        Ok(CapabilitySet(set))
    }

    /// Adds `capability`.
    pub fn insert(&mut self, capability: CapabilityId) -> &mut Self {
        self.0.insert(capability);
        self
    }

    /// Adds `name`, interning it.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::InvalidCapabilityName`] for a name that is not one.
    pub fn with(mut self, name: &str) -> Result<CapabilitySet, RuntimeError> {
        self.0.insert(CapabilityId::intern(name)?);
        Ok(self)
    }

    /// True when the set holds `capability`.
    #[must_use]
    pub fn contains(&self, capability: CapabilityId) -> bool {
        self.0.contains(&capability)
    }

    /// True when the set holds the capability named `name` (an unknown name is held by no set).
    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        CapabilityId::lookup(name).is_some_and(|capability| self.contains(capability))
    }

    /// True when every capability here is also in `other`.
    #[must_use]
    pub fn is_subset_of(&self, other: &CapabilitySet) -> bool {
        self.0.is_subset(&other.0)
    }

    /// The capabilities here that `other` lacks.
    #[must_use]
    pub fn missing_from(&self, other: &CapabilitySet) -> Vec<CapabilityId> {
        self.0.difference(&other.0).copied().collect()
    }

    /// The capabilities, in id order.
    pub fn iter(&self) -> impl Iterator<Item = CapabilityId> + '_ {
        self.0.iter().copied()
    }

    /// How many capabilities the set holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when the set holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for CapabilitySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("{")?;
        for (index, capability) in self.0.iter().enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            f.write_str(capability.name())?;
        }
        f.write_str("}")
    }
}

/// Who granted a set of capabilities.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grantor {
    /// The native host, which holds every capability.
    Root,
    /// A domain delegating a subset of its own grant.
    Domain(DomainId),
}

impl fmt::Display for Grantor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Grantor::Root => f.write_str("root"),
            Grantor::Domain(domain) => write!(f, "{domain}"),
        }
    }
}

/// The host's root authority: it holds every capability and is the only thing that can grant
/// without a parent. Only Rust can make one; a script never sees it.
#[derive(Debug, Default)]
pub struct RootAuthority(());

impl RootAuthority {
    /// The root authority.
    #[must_use]
    pub const fn new() -> RootAuthority {
        RootAuthority(())
    }

    /// Grants `capabilities` outright.
    #[must_use]
    pub fn grant(&self, capabilities: CapabilitySet) -> Grant {
        Grant {
            capabilities: Arc::new(capabilities),
            granted_by: Grantor::Root,
        }
    }
}

/// Capabilities as granted to one domain: the set and who gave it.
#[derive(Clone, Debug)]
pub struct Grant {
    capabilities: Arc<CapabilitySet>,
    granted_by: Grantor,
}

impl Grant {
    /// The granted capabilities.
    #[must_use]
    pub fn capabilities(&self) -> &Arc<CapabilitySet> {
        &self.capabilities
    }

    /// Who granted them.
    #[must_use]
    pub fn granted_by(&self) -> Grantor {
        self.granted_by
    }

    /// A grant of `requested` to a child of `holder`, whose grant this is.
    ///
    /// # Errors
    ///
    /// [`RuntimeError::OverDelegation`] naming every capability the holder lacks.
    pub fn delegate(&self, holder: DomainId, requested: CapabilitySet) -> Result<Grant, RuntimeError> {
        let missing = requested.missing_from(&self.capabilities);
        if !missing.is_empty() {
            return Err(RuntimeError::OverDelegation {
                grantor: Grantor::Domain(holder),
                missing,
            });
        }
        Ok(Grant {
            capabilities: Arc::new(requested),
            granted_by: Grantor::Domain(holder),
        })
    }

    /// The subset `requested` of this grant, for a context or a worker: `None` inherits the
    /// whole grant, `Some` must be a subset or the error names what is missing.
    pub(crate) fn subset(
        &self,
        holder: DomainId,
        requested: Option<CapabilitySet>,
    ) -> Result<Arc<CapabilitySet>, RuntimeError> {
        let Some(requested) = requested else {
            return Ok(Arc::clone(&self.capabilities));
        };
        let missing = requested.missing_from(&self.capabilities);
        if missing.is_empty() {
            Ok(Arc::new(requested))
        } else {
            Err(RuntimeError::OverDelegation {
                grantor: Grantor::Domain(holder),
                missing,
            })
        }
    }
}

/// A capability check that failed: which capability, and who asked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CapabilityDenied {
    /// The capability that was needed.
    pub capability: CapabilityId,
    /// The domain, generation, context, and worker that asked.
    pub attribution: Attribution,
}

impl fmt::Display for CapabilityDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "capability '{}' denied to {}", self.capability.name(), self.attribution)
    }
}

/// Where an authority's denials are reported and its logs go: its domain's sinks and counters.
pub struct Observer {
    pub(crate) diagnostics: Arc<dyn DiagnosticSink>,
    pub(crate) profiler: Arc<dyn ProfileSink>,
    pub(crate) logs: Option<Arc<dyn LogSink>>,
    pub(crate) counters: Arc<Counters>,
}

impl Observer {
    pub(crate) fn denied(&self, denied: &CapabilityDenied) {
        Counters::add(&self.counters.capability_denials, 1);
        self.profiler.record(ProfileEvent::CapabilityDenied {
            attribution: denied.attribution,
            capability: denied.capability,
        });
        self.diagnostics.report(
            Diagnostic::new(
                denied.attribution,
                Severity::Error,
                DiagnosticKind::CapabilityDenied,
                denied.to_string(),
            )
            .capability(denied.capability),
        );
    }
}

/// The authority of the script code running on this thread: its attribution, the capabilities
/// it holds, and the services its domain carries for it. Native services read it through
/// [`current_authority`].
#[derive(Clone)]
pub struct Authority {
    attribution: Attribution,
    capabilities: Arc<CapabilitySet>,
    services: Arc<Services>,
    observer: Option<Arc<Observer>>,
}

impl fmt::Debug for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authority")
            .field("attribution", &self.attribution)
            .field("capabilities", &self.capabilities)
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

impl Authority {
    pub(crate) fn new(
        attribution: Attribution,
        capabilities: Arc<CapabilitySet>,
        services: Arc<Services>,
        observer: Option<Arc<Observer>>,
    ) -> Authority {
        Authority {
            attribution,
            capabilities,
            services,
            observer,
        }
    }

    /// Which domain, generation, context, and worker is running.
    #[must_use]
    pub const fn attribution(&self) -> Attribution {
        self.attribution
    }

    /// The capabilities the running code holds.
    #[must_use]
    pub const fn capabilities(&self) -> &Arc<CapabilitySet> {
        &self.capabilities
    }

    /// True when the running code holds `capability`.
    #[must_use]
    pub fn has(&self, capability: CapabilityId) -> bool {
        self.capabilities.contains(capability)
    }

    /// The services the running code can reach: its domain's, or the ones its worker shares.
    #[must_use]
    pub const fn services(&self) -> &Arc<Services> {
        &self.services
    }

    /// The service of type `T` the running code can reach, if its domain carries one and,
    /// on a worker lane, the worker shares it.
    #[must_use]
    pub fn service<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        self.services.get::<T>()
    }

    /// The log stream of the running code's domain, if it has one; a worker's is its domain's.
    /// A record sent there is attributed with [`Authority::attribution`].
    #[must_use]
    pub fn logs(&self) -> Option<&Arc<dyn LogSink>> {
        self.observer.as_ref()?.logs.as_ref()
    }

    /// Checks that the running code holds `capability`.
    ///
    /// # Errors
    ///
    /// [`CapabilityDenied`] when it does not; the denial is counted and reported to the domain's
    /// diagnostics and profiler before it is returned.
    pub fn require(&self, capability: CapabilityId) -> Result<(), CapabilityDenied> {
        if self.has(capability) {
            return Ok(());
        }
        let denied = CapabilityDenied {
            capability,
            attribution: self.attribution,
        };
        if let Some(observer) = &self.observer {
            observer.denied(&denied);
        }
        Err(denied)
    }
}

thread_local! {
    static CURRENT: RefCell<Vec<Authority>> = const { RefCell::new(Vec::new()) };
}

/// The authority of the script code running on this thread, or `None` outside any domain's
/// dispatch: host code calling into a runtime directly has no authority ticket.
#[must_use]
pub fn current_authority() -> Option<Authority> {
    CURRENT.with_borrow(|stack| stack.last().cloned())
}

/// Checks that the code running on this thread holds the capability `name`: the check a native
/// service makes at its entry point.
///
/// # Errors
///
/// [`RuntimeError::CapabilityDenied`] when it does not, [`RuntimeError::NoAuthority`] outside
/// any dispatch, and [`RuntimeError::InvalidCapabilityName`] for a name that is not one.
pub fn require_capability(name: &str) -> Result<(), RuntimeError> {
    let capability = CapabilityId::intern(name)?;
    let Some(authority) = current_authority() else {
        return Err(RuntimeError::NoAuthority { capability });
    };
    authority.require(capability).map_err(RuntimeError::CapabilityDenied)
}

/// The service of type `T` the code running on this thread can reach.
///
/// That is the one its domain carries ([`DomainSpec::service`](crate::runtime::DomainSpec::service)), or
/// on a worker lane the one its [`WorkerSpec`](crate::runtime::WorkerSpec) shared. `None` when the domain
/// carries no such service, the worker does not share it, or nothing is running under a domain
/// at all: a binding takes `None` outside any dispatch as its cue to fall back to state of its
/// own.
#[must_use]
pub fn service<T: Send + Sync + 'static>() -> Option<Arc<T>> {
    CURRENT.with_borrow(|stack| stack.last().and_then(Authority::service::<T>))
}

/// Installs `authority` as the thread's current one until dropped; nests.
pub struct AuthorityGuard;

impl AuthorityGuard {
    pub(crate) fn enter(authority: Authority) -> AuthorityGuard {
        CURRENT.with_borrow_mut(|stack| stack.push(authority));
        AuthorityGuard
    }
}

impl Drop for AuthorityGuard {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|stack| {
            stack.pop();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ids::Generation;

    #[test]
    fn names_intern_once_and_reject_bad_spellings() {
        let first = CapabilityId::intern("test.intern-once").unwrap();
        let second = CapabilityId::intern("test.intern-once").unwrap();
        assert_eq!(first, second);
        assert_eq!(first.name(), "test.intern-once");
        assert_eq!(CapabilityId::lookup("test.intern-once"), Some(first));
        assert_eq!(CapabilityId::lookup("test.never-interned-anywhere"), None);
        for bad in ["", "Test.echo", "test..echo", "test.", "test echo", "test.é"] {
            assert!(
                matches!(CapabilityId::intern(bad), Err(RuntimeError::InvalidCapabilityName(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn delegation_never_exceeds_the_holder() {
        let root = RootAuthority::new();
        let parent = root.grant(CapabilitySet::from_names(["vfs.read", "input.actions"]).unwrap());
        let holder = DomainId::allocate();
        let child = parent
            .delegate(holder, CapabilitySet::from_names(["vfs.read"]).unwrap())
            .unwrap();
        assert_eq!(child.granted_by(), Grantor::Domain(holder));
        assert!(child.capabilities().contains_name("vfs.read"));
        assert!(!child.capabilities().contains_name("input.actions"));

        let over = parent.delegate(holder, CapabilitySet::from_names(["vfs.read", "render.scene"]).unwrap());
        match over {
            Err(RuntimeError::OverDelegation { grantor, missing }) => {
                assert_eq!(grantor, Grantor::Domain(holder));
                assert_eq!(missing, vec![CapabilityId::intern("render.scene").unwrap()]);
            }
            other => panic!("expected over-delegation, got {other:?}"),
        }
    }

    #[test]
    fn the_current_authority_is_scoped_to_the_thread_and_the_guard() {
        assert!(current_authority().is_none());
        let capability = CapabilityId::intern("test.guarded").unwrap();
        let attribution = Attribution::domain(DomainId::allocate(), Generation::FIRST);
        let mut services = Services::new();
        services.insert(Arc::new(String::from("a service")));
        let authority = Authority::new(
            attribution,
            Arc::new(CapabilitySet::from_names(["test.guarded"]).unwrap()),
            Arc::new(services),
            None,
        );
        assert!(service::<String>().is_none(), "no authority, no service");
        {
            let _guard = AuthorityGuard::enter(authority);
            assert!(current_authority().unwrap().has(capability));
            assert_eq!(service::<String>().as_deref().map(String::as_str), Some("a service"));
            assert!(service::<u64>().is_none(), "a type the domain carries no service of");
            assert!(require_capability("test.guarded").is_ok());
            assert!(matches!(
                require_capability("test.other"),
                Err(RuntimeError::CapabilityDenied(_))
            ));
            let other_thread = std::thread::spawn(|| current_authority().is_none()).join().unwrap();
            assert!(other_thread, "authority never leaks to another thread");
        }
        assert!(matches!(
            require_capability("test.guarded"),
            Err(RuntimeError::NoAuthority { .. })
        ));
        assert!(service::<String>().is_none(), "the guard took the services with it");
    }
}
