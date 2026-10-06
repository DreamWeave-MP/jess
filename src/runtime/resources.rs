//! The ownership registry: what a domain owns, so teardown knows what to release.
//!
//! Anything a native service creates on a domain's behalf (a timer, a file watch, a render
//! target, a callback registration) is registered here with a release function. The domain
//! releases it when asked, when the generation that made it ends, or at teardown, newest first.
//! Workers and contexts are owned the same way, by the domain itself.

use std::fmt;

use crate::runtime::ids::{Generation, ResourceId};

/// One owned resource, as [`ScriptDomain::owned_resources`](crate::runtime::ScriptDomain::owned_resources)
/// lists it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct OwnedResource {
    /// The registration's id.
    pub id: ResourceId,
    /// What kind of thing it is, in the owning service's words.
    pub kind: &'static str,
    /// The generation that created it.
    pub generation: Generation,
}

struct Entry {
    resource: OwnedResource,
    release: Box<dyn FnOnce()>,
}

/// The resources one domain owns, in registration order.
#[derive(Default)]
pub struct ResourceRegistry {
    entries: Vec<Entry>,
}

impl fmt::Debug for ResourceRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| entry.resource))
            .finish()
    }
}

impl ResourceRegistry {
    pub(crate) fn own(
        &mut self,
        kind: &'static str,
        generation: Generation,
        release: impl FnOnce() + 'static,
    ) -> ResourceId {
        let id = ResourceId::allocate();
        self.entries.push(Entry {
            resource: OwnedResource { id, kind, generation },
            release: Box::new(release),
        });
        id
    }

    /// Takes the release function of `id` out of the registry, for the caller to run with no
    /// borrow of the registry held.
    pub(crate) fn take(&mut self, id: ResourceId) -> Option<Box<dyn FnOnce()>> {
        let index = self.entries.iter().position(|entry| entry.resource.id == id)?;
        Some(self.entries.remove(index).release)
    }

    /// Takes every release function, newest first.
    pub(crate) fn take_all(&mut self) -> Vec<Box<dyn FnOnce()>> {
        self.entries.drain(..).rev().map(|entry| entry.release).collect()
    }

    pub(crate) fn list(&self) -> Vec<OwnedResource> {
        self.entries.iter().map(|entry| entry.resource).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn resources_release_newest_first_and_only_once() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut registry = ResourceRegistry::default();
        let note = |name: &'static str| {
            let log = Rc::clone(&log);
            move || log.borrow_mut().push(name)
        };
        let timer = registry.own("timer", Generation::FIRST, note("timer"));
        registry.own("watch", Generation::FIRST, note("watch"));
        registry.own("target", Generation::FIRST.next(), note("target"));
        assert_eq!(registry.list().len(), 3);
        assert_eq!(registry.list()[0].kind, "timer");

        registry.take(timer).expect("the timer is owned")();
        assert!(registry.take(timer).is_none(), "released once");
        for release in registry.take_all() {
            release();
        }
        assert_eq!(*log.borrow(), ["timer", "target", "watch"]);
        assert_eq!(registry.list(), []);
    }
}
