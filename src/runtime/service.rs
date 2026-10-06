//! Services: the native state a domain carries for the code it runs, by type.
//!
//! A host builds a domain around things that outlive any one handler call and belong to the
//! application rather than to a runtime: an input runtime the host ticks, a localization
//! registry the user's locale lives in. The runtime knows nothing about what they are. It only
//! keeps them: [`DomainSpec::service`](crate::runtime::DomainSpec::service) attaches an `Arc<T>` at
//! creation, and a binding running under the domain's authority finds it again with
//! [`service::<T>()`](crate::runtime::service), through the same thread-scoped ticket
//! [`current_authority`](crate::runtime::current_authority) reads.
//!
//! Services are fixed for the domain's life and survive its restarts: they are the host's, not
//! the generation's. A worker sees only the services its [`WorkerSpec`](crate::runtime::WorkerSpec)
//! shares by type; a worker spawned from Luau shares none, because a script cannot name a Rust
//! type, and because a worker exists to run apart from the domain's state.

use std::any::{Any, TypeId, type_name};
use std::fmt;
use std::sync::Arc;

/// One attached service: its type, its type name for people, and the value.
#[derive(Clone)]
struct Entry {
    id: TypeId,
    name: &'static str,
    service: Arc<dyn Any + Send + Sync>,
}

/// The services a domain, or a worker, carries. Looked up by type; a domain carries at most one
/// of each.
#[derive(Clone, Default)]
pub struct Services {
    entries: Vec<Entry>,
}

impl fmt::Debug for Services {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.entries.iter().map(|entry| entry.name)).finish()
    }
}

impl Services {
    /// No services.
    #[must_use]
    pub const fn new() -> Services {
        Services { entries: Vec::new() }
    }

    /// Attaches `service`, replacing one of the same type.
    pub fn insert<T: Send + Sync + 'static>(&mut self, service: Arc<T>) {
        let entry = Entry {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
            service,
        };
        match self.entries.iter_mut().find(|known| known.id == entry.id) {
            Some(known) => *known = entry,
            None => self.entries.push(entry),
        }
    }

    /// The service of type `T`, if one is attached.
    #[must_use]
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let entry = self.entries.iter().find(|entry| entry.id == TypeId::of::<T>())?;
        Arc::clone(&entry.service).downcast::<T>().ok()
    }

    /// True when a service of type `T` is attached.
    #[must_use]
    pub fn has<T: Send + Sync + 'static>(&self) -> bool {
        self.entries.iter().any(|entry| entry.id == TypeId::of::<T>())
    }

    /// The type names of the attached services, in attachment order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|entry| entry.name).collect()
    }

    /// How many services are attached.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is attached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The services a worker shares (as [`WorkerSpec::share`](crate::runtime::WorkerSpec::share) and
    /// [`WorkerSpec::share_all`](crate::runtime::WorkerSpec::share_all) said), or the name of the first
    /// one this set lacks.
    pub(crate) fn shared(&self, sharing: &crate::runtime::worker::Sharing) -> Result<Services, &'static str> {
        let shared: &[Shared] = match sharing {
            crate::runtime::worker::Sharing::None => &[],
            crate::runtime::worker::Sharing::Some(shared) => shared,
            crate::runtime::worker::Sharing::All => return Ok(self.clone()),
        };
        let mut subset = Services::new();
        for wanted in shared {
            let entry = self.entries.iter().find(|entry| entry.id == wanted.id).ok_or(wanted.name)?;
            subset.entries.push(Entry {
                id: entry.id,
                name: entry.name,
                service: Arc::clone(&entry.service),
            });
        }
        Ok(subset)
    }
}

/// A service type a worker asks to share, before anything checks the domain has it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Shared {
    pub(crate) id: TypeId,
    pub(crate) name: &'static str,
}

impl Shared {
    pub(crate) fn of<T: Send + Sync + 'static>() -> Shared {
        Shared {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Clock(u64);
    struct Names(Vec<String>);

    #[test]
    fn services_are_one_per_type_and_found_by_type() {
        let mut services = Services::new();
        assert!(services.is_empty());
        services.insert(Arc::new(Clock(1)));
        services.insert(Arc::new(Names(vec!["a".to_owned()])));
        services.insert(Arc::new(Clock(2)));
        assert_eq!(services.len(), 2);
        assert_eq!(services.get::<Clock>().unwrap().0, 2, "the later attachment replaces");
        assert_eq!(services.get::<Names>().unwrap().0, ["a"]);
        assert!(services.has::<Clock>() && !services.has::<String>());
        assert_eq!(services.get::<String>(), None);
        assert_eq!(services.names(), [type_name::<Clock>(), type_name::<Names>()]);
    }

    #[test]
    fn a_subset_holds_only_what_was_shared_and_names_what_is_missing() {
        use crate::runtime::worker::Sharing;

        let mut services = Services::new();
        services.insert(Arc::new(Clock(7)));

        let subset = services.shared(&Sharing::Some(vec![Shared::of::<Clock>()])).unwrap();
        assert_eq!(subset.get::<Clock>().unwrap().0, 7);
        assert_eq!(services.shared(&Sharing::None).unwrap().len(), 0);
        assert_eq!(services.shared(&Sharing::All).unwrap().len(), 1);
        let missing = services
            .shared(&Sharing::Some(vec![Shared::of::<Clock>(), Shared::of::<Names>()]))
            .map(|subset| subset.len());
        assert_eq!(missing, Err(type_name::<Names>()));
    }
}
