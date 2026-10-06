//! The opaque identities the runtime hands out.
//!
//! Every id is an integer a registry allocated, never a pointer: it stays meaningful in a
//! diagnostic after the thing it named is gone, and it can cross threads. Ids are unique for the
//! life of the process, so a `ContextId` alone names one context of one domain, and a profiler
//! line never needs the domain beside it to be unambiguous.

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub struct $name(u32);

        impl $name {
            /// The next unused id. Ids count up from 1 and are never reused.
            pub(crate) fn allocate() -> Self {
                static NEXT: AtomicU32 = AtomicU32::new(1);
                let raw = NEXT.fetch_add(1, Ordering::Relaxed);
                assert!(raw != u32::MAX, concat!("ran out of ", $prefix, " ids"));
                Self(raw)
            }

            /// The id's integer, for hosts that key their own tables by it.
            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "#{}"), self.0)
            }
        }
    };
}

id_type! {
    /// One [`ScriptDomain`](crate::runtime::ScriptDomain), for its whole life across every generation.
    DomainId, "domain"
}

id_type! {
    /// One worker of one domain.
    WorkerId, "worker"
}

id_type! {
    /// One context ([`ContextSpec`](crate::runtime::ContextSpec)) of one domain.
    ContextId, "context"
}

id_type! {
    /// One native resource a domain owns through its ownership registry.
    ResourceId, "resource"
}

/// One capability name, interned by [`CapabilityId::intern`](crate::runtime::CapabilityId::intern);
/// equal ids are equal names.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct CapabilityId(pub(crate) u32);

impl CapabilityId {
    /// The id's integer.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Which incarnation of a domain's code a piece of work belongs to. A domain starts at
/// [`Generation::FIRST`] and every restart moves it on; work queued for one generation is never
/// delivered to the next.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Generation(u64);

impl Generation {
    /// The generation a domain is created in.
    pub const FIRST: Generation = Generation(1);

    /// The generation after this one.
    #[must_use]
    pub const fn next(self) -> Generation {
        Generation(self.0 + 1)
    }

    /// The generation's integer.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }

    pub(crate) const fn from_raw(raw: u64) -> Generation {
        Generation(raw)
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gen#{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_display_with_their_prefix() {
        let first = DomainId::allocate();
        let second = DomainId::allocate();
        assert_ne!(first, second);
        assert!(second > first);
        assert_eq!(first.to_string(), format!("domain#{}", first.raw()));
        assert_eq!(WorkerId::allocate().to_string().split('#').next(), Some("worker"));
        assert_eq!(ContextId::allocate().to_string().split('#').next(), Some("context"));
    }

    #[test]
    fn generations_count_up_from_one() {
        assert_eq!(Generation::FIRST.raw(), 1);
        assert_eq!(Generation::FIRST.next().raw(), 2);
        assert_eq!(Generation::FIRST.next().to_string(), "gen#2");
        assert!(Generation::FIRST < Generation::FIRST.next());
    }
}
