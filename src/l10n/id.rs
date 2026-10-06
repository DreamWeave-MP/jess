//! The registry's identities: opaque integers it allocates, never pointers.

use std::fmt;

/// A locale a [`Registry`](crate::l10n::Registry) has interned.
///
/// Ids are dense and allocated in the order locales are first seen; one is meaningful only to
/// the registry that made it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct LocaleId(pub(crate) u32);

/// A named bundle of messages in a [`Registry`](crate::l10n::Registry).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct BundleId(pub(crate) u32);

/// A message name interned in one bundle: the bundle and the name's index in it.
///
/// An id is stable for the registry's life: reloads and locale changes never renumber it, and it
/// exists whether or not any locale currently defines the message.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct MessageId(pub(crate) u64);

/// A provider registered with a [`Registry`](crate::l10n::Registry). A higher id was registered later
/// and takes precedence.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ProviderId(pub(crate) u32);

/// A point in a registry's history. It rises by one on every change to what formatting returns:
/// a reload that swaps a bundle in, a new locale preference, a new fallback locale.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct Generation(pub(crate) u64);

impl MessageId {
    pub(crate) const fn new(bundle: BundleId, index: u32) -> Self {
        Self(((bundle.0 as u64) << 32) | index as u64)
    }

    /// The bundle the message belongs to.
    #[must_use]
    pub const fn bundle(self) -> BundleId {
        #[allow(clippy::cast_possible_truncation)] // The high half, by construction.
        BundleId((self.0 >> 32) as u32)
    }

    pub(crate) const fn index(self) -> usize {
        (self.0 & 0xFFFF_FFFF) as usize
    }
}

impl Generation {
    /// The generation as a number, for logs and for comparing across a boundary that can't carry
    /// the type.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl fmt::Display for LocaleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "locale#{}", self.0)
    }
}

impl fmt::Display for BundleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bundle#{}", self.0)
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "message#{}:{}", self.bundle().0, self.index())
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "provider#{}", self.0)
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "generation {}", self.0)
    }
}
